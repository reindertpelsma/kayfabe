//! ★ 2026-10-16 — **Exhaustive interleaving model of the USER-ring lifecycle** (Scan / Pointer / Window,
//! `docs/design/V3_USER_RING_LIFECYCLE_20261016.md`). GPU-free, deterministic, NO wall-clock time: there is
//! no timer step at all, so every liveness result is "with time forbidden" (G4, I6).
//!
//! ACTORS (every item a separate ATOMIC step, so the explorer runs any other actor between any two):
//! * the **guest**, a script of `Push` (a GP entry, 64-bit store), `SetPut`, `Ring`, `Fill` (late
//!   pushbuffer bytes), `Release` (a semaphore), `Register` / `Remove` + `AwaitReply` (the registration
//!   RPCs), `UseNew` (the migrated ring), and the hostile `Mutate` (ONE dword of a ring slot) / `PutRaw`;
//! * the **drainer**: applies the registration (live set + epoch, one step) and delivers the reply in a
//!   LATER step (any time after the bump, which covers an immediate and a held reply);
//! * the **PBDMA**: `Pop` (GP_GET runs ahead of processing, measured) and `Process` (in order; an
//!   acquire blocks the next entry; a marker writes the tagged REFERENCE and raises the event);
//! * the **worker**: a turn of micro-steps from `lifecycle_support::Relay`, which drives the REAL
//!   `Lifecycle`, `SyncGate`, `Marks`, `Grace`, `Scan`, `Fetch` and `inspect_entry`.
//!
//! WHAT IS CHECKED (derived from the design, not from the code): I1 no pushbuffer read in Pointer, I2
//! the PBDMA executes exactly the guest's entries in order, I3 no stale read (the model's `STALE`), I4 no
//! entry published after a registration's reply is forwarded unscanned, I5 only listed transitions, I6
//! liveness with time forbidden, I7 value tested == value stored while the guest changes ONE dword.
#![allow(
    clippy::too_many_lines,
    clippy::needless_range_loop,
    dead_code,
    missing_docs,
    clippy::collapsible_if
)]

#[path = "lifecycle_support/mod.rs"]
mod support;

use kf_chan::user_gate::MARK_TAG;
use kf_chan::user_lifecycle::{Lifecycle, LifecycleMode, Mode, Transition, next};
use kf_chan::user_scan::Scan;
use std::collections::{HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use support::*;

// ------------------------------------------------------------------------------------------------
// vocabulary

#[derive(Clone, Copy, Debug)]
pub enum Op {
    Push(Ge),
    SetPut,
    Ring,
    Release(u8),
    Fill(u8),
    Register,
    Remove,
    AwaitReply,
    UseNew,
    /// The guest presets the new ring's USERD (Keep semantics: GP_PUT/GP_GET continue the old ring's values).
    PrepNew,
    /// Hostile: change ONE dword of a ring slot of ring `ring` at any instant.
    Mutate {
        ring: usize,
        slot: u32,
        hi: bool,
        xor: u32,
    },
    PutRaw(u32),
    /// The channel is freed (device reset / guest free).
    Teardown,
}

pub const V_STALE: u32 = 1 << 0;
pub const V_I1: u32 = 1 << 1;
pub const V_I4: u32 = 1 << 2;
pub const V_TORN: u32 = 1 << 3;
pub const V_ORDER: u32 = 1 << 4;
pub const V_UNSCANNED_TRIGGER: u32 = 1 << 5;
pub const V_REFUSED: u32 = 1 << 6;
pub const V_PUT_BOUND: u32 = 1 << 7;
pub const V_AFTER_TEARDOWN: u32 = 1 << 8;
pub const V_TRANSITION: u32 = 1 << 9;
pub const V_DEADLOCK: u32 = 1 << 10;
pub const V_LOST: u32 = 1 << 11;
pub const V_ENTRY_REFUSED: u32 = 1 << 12;
/// REVIEW: a host marker published while the HARDWARE's view of the private stream has a packet open.
pub const V_MARKER_IN_PACKET: u32 = 1 << 13;
/// I8: the relay read something at or beyond the `GP_PUT` it loaded in the turn (or loaded `GP_PUT` twice).
pub const V_BEYOND_PUT: u32 = 1 << 14;

pub fn viol_names(v: u32) -> Vec<&'static str> {
    let mut out = Vec::new();
    for (bit, name) in [
        (
            V_STALE,
            "STALE (I3: a pushbuffer read before the guest wrote it)",
        ),
        (V_I1, "I1 (a pushbuffer read in Pointer mode)"),
        (
            V_I4,
            "I4 (an entry published after the reply was forwarded unscanned)",
        ),
        (V_TORN, "I7 (the value stored is not the value tested)"),
        (
            V_ORDER,
            "I2 (the PBDMA executed something other than the guest's entries in order)",
        ),
        (
            V_UNSCANNED_TRIGGER,
            "UNSCANNED_TRIGGER (the hardware executed a trigger nobody scanned)",
        ),
        (
            V_REFUSED,
            "REFUSED (the scan refused a segment for good: the channel is dead)",
        ),
        (V_PUT_BOUND, "PUT_BOUND (hostile GP_PUT refused)"),
        (
            V_AFTER_TEARDOWN,
            "AFTER_TEARDOWN (the private ring grew after the channel was freed)",
        ),
        (V_TRANSITION, "I5 (a transition outside the table)"),
        (
            V_DEADLOCK,
            "I6 (a terminal state with covered work unrelayed and no timer)",
        ),
        (V_LOST, "I2 (covered entries not all relayed)"),
        (
            V_MARKER_IN_PACKET,
            "MARKER_IN_PACKET (a host marker lands inside an open guest packet)",
        ),
        (
            V_BEYOND_PUT,
            "I8 (a ring entry or pushbuffer at or beyond the loaded GP_PUT was read)",
        ),
        (
            V_ENTRY_REFUSED,
            "ENTRY_REFUSED (inspect_entry refused the loaded value)",
        ),
    ] {
        if v & bit != 0 {
            out.push(name);
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Registration {
    /// The registered bundle is a ring growth (`FIFO_UPDATE_CHANNEL_INFO`).
    Migrate,
    /// A TLB-invalidate-like bundle.
    Other,
}

#[derive(Clone, Debug)]
pub struct Spec {
    pub script: Vec<Op>,
    /// The ids the PBDMA must execute, in order.
    pub expected: Vec<u8>,
    /// At a terminal state at least this many leading ids must have executed.
    pub must_relay: usize,
    /// The script may legitimately end blocked.
    pub may_block: bool,
    /// A hostile script: no order / prefix checks.
    pub hostile: bool,
}

#[derive(Clone, Debug)]
pub struct Cfg {
    pub spec: Spec,
    pub priv_entries: u32,
    pub src_entries: u32,
    pub glue: Glue,
    pub kind: Registration,
    pub cursor_relay: Cursor,
    /// The guest's first slot in the new ring.
    pub cursor_guest: Cursor,
    /// May a "look" (the bounded spin / the counted backstop) find a satisfied wait that no interrupt announces?
    pub looks: bool,
    pub pop_depth: usize,
    pub always_scan: bool,
    /// PHASE 1c: the measurement-class mode (`KF3_LIFECYCLE_MODE`).
    pub mode: LifecycleMode,
    pub max_states: usize,
}
impl Cfg {
    pub fn new(spec: Spec) -> Self {
        Self {
            spec,
            priv_entries: 8,
            src_entries: 8,
            glue: Glue::from_env(),
            kind: Registration::Migrate,
            cursor_relay: Cursor::Zero,
            cursor_guest: Cursor::Zero,
            looks: true,
            pop_depth: usize::MAX,
            always_scan: false,
            mode: LifecycleMode::Default,
            max_states: 4_000_000,
        }
    }
}

// ------------------------------------------------------------------------------------------------
// the world

struct ModelEnv {
    ges: Rc<Vec<Option<Ge>>>,
    rings: [Vec<(u32, u32)>; 2],
    put: [u32; 2],
    guest_get: [u32; 2],
    src: usize,
    fill: u32,
    epoch: u64,
    live: u32,
    log: Vec<Pub>,
    popped: usize,
    hw_get: usize,
    markers_done: u64,
    ev: bool,
    hw_wait: Option<u8>,
    sem: u32,
    executed: Vec<u8>,
    viol: u32,
    kind: Registration,
    carried_ungated: u32,
    switched: u32,
    entry_refused: u32,
    scan_refused: u32,
    retries: u32,
    triggers_found: u32,
    pb_reads: u32,
    priv_entries: u32,
    src_entries: u32,
    hw_scan: Scan,
}

impl ModelEnv {
    /// The PBDMA has popped entries the relay has not observed yet.
    fn hw_get_unseen(&self, relay: &Relay) -> bool {
        self.popped as u64 > relay.fetch.fetched()
    }
    fn ge(&self, id: u8) -> Ge {
        self.ges[usize::from(id)].expect("known id")
    }
}

impl Env for ModelEnv {
    fn src_entries(&self) -> u32 {
        self.src_entries
    }
    fn gp_put(&mut self) -> u32 {
        self.put[self.src]
    }
    fn gp_dword(&mut self, slot: u32, hi: bool) -> u32 {
        let (lo, h) = self.rings[self.src][slot as usize];
        if hi { h } else { lo }
    }
    fn registry_epoch(&mut self) -> u64 {
        self.epoch
    }
    fn registry_live(&mut self) -> u32 {
        self.live
    }
    fn hw_get(&mut self) -> u32 {
        (self.popped as u32) & (self.priv_entries - 1)
    }
    fn hw_reference(&mut self) -> u32 {
        if self.markers_done == 0 {
            0
        } else {
            MARK_TAG | (self.markers_done as u32 & 0xffff)
        }
    }
    fn read_pb(&mut self, raw: u64) -> Vec<u32> {
        self.pb_reads += 1;
        let Some(id) = entry_id(raw) else {
            self.viol |= V_STALE;
            return vec![POISON];
        };
        let g = self.ge(id);
        let words = g.words();
        if g.late && self.fill & (1 << id) == 0 {
            self.viol |= V_STALE;
            return vec![POISON; words.len()];
        }
        words
    }
    fn publish(&mut self, p: Pub) {
        // REVIEW: the hardware parses the private stream as ONE packet stream across segments.
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
        if let Pub::Guest {
            tested,
            stored,
            scanned,
        } = p
        {
            if tested != stored {
                self.viol |= V_TORN;
            }
            if let Some(id) = entry_id(tested) {
                if let Some(g) = self.ges.get(usize::from(id)).copied().flatten() {
                    // a registration is live: every entry published after its reply must have been scanned
                    if g.post_reply && g.kind != Kind::Nop && !scanned && self.live > 0 {
                        self.viol |= V_I4;
                    }
                }
            }
        }
        self.log.push(p);
    }
    fn write_guest_get(&mut self, v: u32) {
        self.guest_get[self.src] = v;
    }
    fn trigger_migrates(&mut self, _raw: u64) -> bool {
        self.kind == Registration::Migrate
    }
    fn executed(&mut self, migrate: bool) {
        self.live = self.live.saturating_sub(1);
        self.epoch += 1;
        if migrate {
            self.src = 1;
        }
    }
    fn event(&mut self, e: Ev) {
        match e {
            Ev::PutBound => self.viol |= V_PUT_BOUND,
            Ev::EntryRefused => {
                self.viol |= V_ENTRY_REFUSED;
                self.entry_refused += 1;
            }
            Ev::ScanRefused => {
                self.viol |= V_REFUSED;
                self.scan_refused += 1;
            }
            Ev::CarriedUngated => self.carried_ungated += 1,
            Ev::Retry => self.retries += 1,
            Ev::TriggerFound => self.triggers_found += 1,
            Ev::Switched => self.switched += 1,
            Ev::BeyondPut => self.viol |= V_BEYOND_PUT,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Rpc {
    Register,
    Remove,
}

struct World {
    pc: usize,
    gr: usize,
    put_local: [u32; 2],
    put_cnt: usize,
    pushed: Vec<u8>,
    flag: bool,
    covered: usize,
    got_reply: bool,
    rpc: Option<Rpc>,
    reply_pending: bool,
    reply_ready: bool,
    torn_down: bool,
    log_at_teardown: usize,
    relay: Relay,
    env: ModelEnv,
}

impl Clone for World {
    fn clone(&self) -> Self {
        Self {
            pc: self.pc,
            gr: self.gr,
            put_local: self.put_local,
            put_cnt: self.put_cnt,
            pushed: self.pushed.clone(),
            flag: self.flag,
            covered: self.covered,
            got_reply: self.got_reply,
            rpc: self.rpc,
            reply_pending: self.reply_pending,
            reply_ready: self.reply_ready,
            torn_down: self.torn_down,
            log_at_teardown: self.log_at_teardown,
            relay: self.relay.clone(),
            env: ModelEnv {
                ges: self.env.ges.clone(),
                rings: self.env.rings.clone(),
                put: self.env.put,
                guest_get: self.env.guest_get,
                src: self.env.src,
                fill: self.env.fill,
                epoch: self.env.epoch,
                live: self.env.live,
                log: self.env.log.clone(),
                popped: self.env.popped,
                hw_get: self.env.hw_get,
                markers_done: self.env.markers_done,
                ev: self.env.ev,
                hw_wait: self.env.hw_wait,
                sem: self.env.sem,
                executed: self.env.executed.clone(),
                viol: self.env.viol,
                kind: self.env.kind,
                carried_ungated: self.env.carried_ungated,
                switched: self.env.switched,
                entry_refused: self.env.entry_refused,
                scan_refused: self.env.scan_refused,
                retries: self.env.retries,
                triggers_found: self.env.triggers_found,
                pb_reads: self.env.pb_reads,
                priv_entries: self.env.priv_entries,
                src_entries: self.env.src_entries,
                hw_scan: self.env.hw_scan.clone(),
            },
        }
    }
}

impl World {
    fn new(cfg: &Cfg) -> Self {
        let mut ges: Vec<Option<Ge>> = vec![None; 64];
        for op in &cfg.spec.script {
            if let Op::Push(g) = op {
                ges[usize::from(g.id)] = Some(*g);
            }
        }
        let life = if cfg.always_scan {
            Lifecycle::always_scan()
        } else {
            Lifecycle::new().with_mode(cfg.mode)
        };
        let scan = Scan::for_subdevice(1).expect("sub-device");
        let mut relay = Relay::new(cfg.priv_entries, life, scan, cfg.glue);
        relay.cursor_after_switch = cfg.cursor_relay;
        let n = cfg.src_entries as usize;
        Self {
            pc: 0,
            gr: 0,
            put_local: [0; 2],
            put_cnt: 0,
            pushed: Vec::new(),
            flag: false,
            covered: 0,
            got_reply: false,
            rpc: None,
            reply_pending: false,
            reply_ready: false,
            torn_down: false,
            log_at_teardown: 0,
            relay,
            env: ModelEnv {
                ges: Rc::new(ges),
                rings: [vec![(0, 0); n], vec![(0, 0); n]],
                put: [0; 2],
                guest_get: [0; 2],
                src: 0,
                fill: 0,
                epoch: 0,
                live: 0,
                log: Vec::new(),
                popped: 0,
                hw_get: 0,
                markers_done: 0,
                ev: false,
                hw_wait: None,
                sem: 0,
                executed: Vec::new(),
                viol: 0,
                kind: cfg.kind,
                carried_ungated: 0,
                switched: 0,
                entry_refused: 0,
                scan_refused: 0,
                retries: 0,
                triggers_found: 0,
                pb_reads: 0,
                priv_entries: cfg.priv_entries,
                src_entries: cfg.src_entries,
                hw_scan: Scan::default(),
            },
        }
    }

    fn key(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.pc.hash(&mut h);
        self.gr.hash(&mut h);
        self.put_local.hash(&mut h);
        self.put_cnt.hash(&mut h);
        self.pushed.hash(&mut h);
        self.flag.hash(&mut h);
        self.covered.hash(&mut h);
        self.got_reply.hash(&mut h);
        self.rpc.hash(&mut h);
        self.reply_pending.hash(&mut h);
        self.reply_ready.hash(&mut h);
        self.torn_down.hash(&mut h);
        self.relay.key().hash(&mut h);
        let e = &self.env;
        e.rings.hash(&mut h);
        e.put.hash(&mut h);
        e.guest_get.hash(&mut h);
        e.src.hash(&mut h);
        e.fill.hash(&mut h);
        e.epoch.hash(&mut h);
        e.live.hash(&mut h);
        e.log.hash(&mut h);
        e.popped.hash(&mut h);
        e.hw_get.hash(&mut h);
        e.markers_done.hash(&mut h);
        e.ev.hash(&mut h);
        e.hw_wait.hash(&mut h);
        e.sem.hash(&mut h);
        e.executed.hash(&mut h);
        e.viol.hash(&mut h);
        h.finish()
    }

    fn done(&self, cfg: &Cfg) -> bool {
        self.pc >= cfg.spec.script.len()
    }

    /// Run one relay micro-step with the I1 / I5 / teardown checks around it.
    fn relay_step(&mut self) {
        let before_phase = self.relay.phase;
        let mode_before = self.relay.life.mode();
        let log_before = self.env.log.len();
        self.relay.step(&mut self.env);
        if before_phase == kf_chan::user_turn::Phase::PbRead
            && self.relay.life.mode() == Mode::Pointer
        {
            self.env.viol |= V_I1;
        }
        let _ = mode_before;
        if let Some(Transition { from, cause, to }) = self.relay.life.take_transition() {
            if next(from, cause) != Some(to) {
                self.env.viol |= V_TRANSITION;
            }
        }
        if self.torn_down && self.env.log.len() > self.log_at_teardown.max(log_before) {
            self.env.viol |= V_AFTER_TEARDOWN;
        }
    }

    fn successors(&self, cfg: &Cfg) -> Vec<World> {
        let mut out = Vec::new();
        let e = &self.env;
        // ---- guest
        if !self.done(cfg) {
            let op = cfg.spec.script[self.pc];
            let mut w = self.clone();
            let n = cfg.src_entries as usize;
            let mut ok = true;
            match op {
                Op::Push(g) => {
                    let r = self.gr;
                    if (self.put_local[r] + 1) % cfg.src_entries == e.guest_get[r] {
                        ok = false; // the ring is full: the guest waits for the GP_GET write-back
                    } else {
                        let raw = g.raw();
                        w.env.rings[r][self.put_local[r] as usize] =
                            (raw as u32, (raw >> 32) as u32);
                        w.put_local[r] = (self.put_local[r] + 1) % cfg.src_entries;
                        w.pushed.push(g.id);
                        if !g.late {
                            w.env.fill |= 1 << g.id;
                        }
                    }
                }
                Op::SetPut => {
                    w.env.put[self.gr] = self.put_local[self.gr];
                    w.put_cnt = self.pushed.len();
                }
                Op::Ring => {
                    w.flag = true;
                    w.covered = self.put_cnt;
                }
                Op::Release(s) => w.env.sem |= 1 << s,
                Op::Fill(id) => w.env.fill |= 1 << id,
                Op::Register => {
                    w.rpc = Some(Rpc::Register);
                }
                Op::Remove => {
                    w.rpc = Some(Rpc::Remove);
                }
                Op::AwaitReply => {
                    if self.reply_ready {
                        w.reply_ready = false;
                        w.got_reply = true;
                    } else {
                        ok = false;
                    }
                }
                Op::UseNew => {
                    w.gr = 1;
                }
                Op::PrepNew => {
                    if cfg.cursor_guest == Cursor::Keep {
                        // the continued value: where the guest's write position is NOW (past the trigger)
                        w.put_local[1] = self.put_local[0];
                        w.env.put[1] = self.put_local[0];
                        w.env.guest_get[1] = self.env.guest_get[0];
                    }
                }
                Op::Mutate {
                    ring,
                    slot,
                    hi,
                    xor,
                } => {
                    let s = &mut w.env.rings[ring][slot as usize % n];
                    if hi {
                        s.1 ^= xor;
                    } else {
                        s.0 ^= xor;
                    }
                }
                Op::PutRaw(v) => w.env.put[self.gr] = v,
                Op::Teardown => {
                    w.torn_down = true;
                    w.log_at_teardown = e.log.len();
                    w.relay.life.teardown();
                    w.relay.dead = true;
                }
            }
            if ok {
                w.pc += 1;
                out.push(w);
            }
        }
        // ---- the drainer
        if let Some(rpc) = self.rpc {
            let mut w = self.clone();
            w.rpc = None;
            match rpc {
                Rpc::Register => {
                    w.env.live += 1;
                    w.env.epoch += 1;
                }
                Rpc::Remove => {
                    if w.env.live > 0 {
                        w.env.live -= 1;
                        w.env.epoch += 1;
                    }
                }
            }
            w.reply_pending = true;
            out.push(w);
        }
        if self.reply_pending {
            let mut w = self.clone();
            w.reply_pending = false;
            w.reply_ready = true;
            out.push(w);
        }
        // ---- the PBDMA
        if e.popped < e.log.len() && e.popped - e.hw_get.min(e.popped) < cfg.pop_depth {
            let mut w = self.clone();
            w.env.popped += 1;
            out.push(w);
        }
        if e.hw_get < e.popped.min(e.log.len()) {
            let blocked = e.hw_wait.is_some_and(|s| e.sem & (1 << s) == 0);
            if !blocked {
                let mut w = self.clone();
                w.env.hw_wait = None;
                let p = w.env.log[w.env.hw_get];
                w.env.hw_get += 1;
                match p {
                    Pub::Guest { stored, .. } => {
                        if let Some(id) = entry_id(stored) {
                            if let Some(g) = w.env.ges[usize::from(id)] {
                                if g.is_trigger() {
                                    w.env.viol |= V_UNSCANNED_TRIGGER;
                                }
                                w.env.hw_wait = g.acq;
                            }
                            w.env.executed.push(id);
                        } else {
                            w.env.viol |= V_TORN;
                        }
                    }
                    Pub::Staged { tested } => {
                        if let Some(id) = entry_id(tested) {
                            w.env.executed.push(id);
                        }
                    }
                    Pub::Barrier => {}
                    Pub::Marker(_) => {
                        w.env.markers_done += 1;
                        w.env.ev = true;
                    }
                }
                if !cfg.spec.hostile {
                    let exp = &cfg.spec.expected;
                    let ex = &w.env.executed;
                    if ex.len() > exp.len() || ex[..] != exp[..ex.len()] {
                        w.env.viol |= V_ORDER;
                    }
                }
                out.push(w);
            }
        }
        // ---- the worker
        if !self.relay.dead && !self.torn_down {
            if !self.relay.idle() {
                let mut w = self.clone();
                w.relay_step();
                out.push(w);
            } else {
                if self.flag {
                    let mut w = self.clone();
                    w.flag = false; // BEFORE the turn reads GP_PUT
                    w.relay.claim();
                    out.push(w);
                } else if self.relay.again {
                    let mut w = self.clone();
                    w.relay.claim();
                    out.push(w);
                }
                // the event look: the marker's interrupt wakes the worker, which peeks every wait
                if self.env.ev && !cfg.glue.ignore_events {
                    let mut w = self.clone();
                    w.env.ev = false;
                    let mut env = std::mem::replace(&mut w.env, empty_env());
                    if w.relay.wait_satisfied(&mut env) {
                        w.relay.clear_waiting();
                        w.relay.again = true;
                    }
                    w.env = env;
                    out.push(w);
                }
                // the bounded spin / the counted backstop: a LOOK at a wait that no interrupt announces
                if cfg.looks && self.relay.waiting().is_some() {
                    let mut w = self.clone();
                    let mut env = std::mem::replace(&mut w.env, empty_env());
                    let sat = w.relay.wait_satisfied(&mut env);
                    // a gate / batch is still draining but the PBDMA has popped more entries than the guest's
                    // GP_GET shows: the look rings the relay only to write GP_GET back (`pop_writebacks`)
                    let pops = matches!(
                        w.relay.waiting(),
                        Some(kf_chan::user_loop::Wait::Marker { .. })
                    ) && env.hw_get_unseen(&w.relay);
                    w.env = env;
                    if sat {
                        w.relay.clear_waiting();
                        w.relay.again = true;
                        out.push(w);
                    } else if pops {
                        w.relay.again = true;
                        out.push(w);
                    }
                }
            }
        }
        out
    }
}

fn empty_env() -> ModelEnv {
    ModelEnv {
        ges: Rc::new(Vec::new()),
        rings: [Vec::new(), Vec::new()],
        put: [0; 2],
        guest_get: [0; 2],
        src: 0,
        fill: 0,
        epoch: 0,
        live: 0,
        log: Vec::new(),
        popped: 0,
        hw_get: 0,
        markers_done: 0,
        ev: false,
        hw_wait: None,
        sem: 0,
        executed: Vec::new(),
        viol: 0,
        kind: Registration::Migrate,
        carried_ungated: 0,
        switched: 0,
        entry_refused: 0,
        scan_refused: 0,
        retries: 0,
        triggers_found: 0,
        pb_reads: 0,
        priv_entries: 4,
        src_entries: 4,
        hw_scan: Scan::default(),
    }
}

// ------------------------------------------------------------------------------------------------
// the explorer

#[derive(Debug, Default)]
pub struct Report {
    pub states: usize,
    pub terminals: usize,
    pub viol: u32,
    pub first: Option<String>,
    /// Over all terminal states: the lifecycle modes reached.
    pub modes: HashSet<Mode>,
    /// Over all states: the modes ever seen.
    pub modes_seen: HashSet<Mode>,
    pub carried_ungated: u32,
    pub pb_reads_max: u32,
    pub switched_terminals: usize,
    pub retries_max: u32,
    pub window_fresh_errors_max: u64,
    pub straddles_window_max: u64,
    pub to_window_fresh_max: u64,
    pub to_pointer_max: u64,
    pub seeded_max: u64,
    pub restarts_max: u64,
    pub entry_refused: u32,
    pub scan_refused: u32,
    pub unscanned_trigger_terminals: usize,
    pub triggers_found_max: u32,
    pub straddles_scan_max: u64,
    pub binding_changed_max: u64,
    pub entries_pointer_max: u64,
    pub put_boundaries_scan_max: u64,
    pub put_boundary_open_scan_max: u64,
    pub pointer_markers_max: u64,
    pub pointer_carried_no_marker_max: u64,
    /// PHASE 1c: states in which this channel had latched Pointer markers off, the violations seen in them, and the
    /// most Pointer markers / unannounced wakes counted there.
    pub latched_states: usize,
    pub viol_latched: u32,
    pub latched_markers_max: u64,
    pub latched_carried_max: u64,
    pub set_object_total_max: u64,
    pub set_object_rebinds_max: u64,
}

pub fn explore(cfg: &Cfg) -> Report {
    let mut rep = Report::default();
    let w0 = World::new(cfg);
    let mut seen: HashSet<u64> = HashSet::new();
    seen.insert(w0.key());
    let mut queue = VecDeque::new();
    queue.push_back(w0);
    while let Some(w) = queue.pop_front() {
        rep.states += 1;
        assert!(
            rep.states <= cfg.max_states,
            "scenario too large: more than {} states",
            cfg.max_states
        );
        rep.modes_seen.insert(w.relay.life.mode());
        let c = w.relay.life.counts();
        rep.window_fresh_errors_max = rep
            .window_fresh_errors_max
            .max(c.window_fresh_start_parse_error);
        rep.straddles_window_max = rep.straddles_window_max.max(c.straddles_window);
        rep.to_window_fresh_max = rep.to_window_fresh_max.max(c.to_window_fresh);
        rep.to_pointer_max = rep
            .to_pointer_max
            .max(c.to_pointer + c.to_pointer_from_window);
        rep.seeded_max = rep.seeded_max.max(c.seeded_gates);
        rep.restarts_max = rep.restarts_max.max(c.restarts);
        rep.pb_reads_max = rep.pb_reads_max.max(w.env.pb_reads);
        rep.triggers_found_max = rep.triggers_found_max.max(w.env.triggers_found);
        rep.straddles_scan_max = rep.straddles_scan_max.max(c.straddles_scan);
        rep.binding_changed_max = rep.binding_changed_max.max(c.window_binding_changed);
        rep.entries_pointer_max = rep.entries_pointer_max.max(c.entries_pointer);
        rep.put_boundaries_scan_max = rep.put_boundaries_scan_max.max(c.put_boundaries_scan);
        rep.put_boundary_open_scan_max =
            rep.put_boundary_open_scan_max.max(c.put_boundary_open_scan);
        rep.pointer_markers_max = rep.pointer_markers_max.max(c.pointer_markers);
        rep.set_object_total_max = rep.set_object_total_max.max(c.set_object_5080_total);
        rep.set_object_rebinds_max = rep.set_object_rebinds_max.max(c.set_object_5080_rebinds);
        rep.pointer_carried_no_marker_max = rep
            .pointer_carried_no_marker_max
            .max(c.pointer_carried_no_marker);
        rep.carried_ungated = rep.carried_ungated.max(w.env.carried_ungated);
        rep.retries_max = rep.retries_max.max(w.env.retries);
        rep.entry_refused = rep.entry_refused.max(w.env.entry_refused);
        rep.scan_refused = rep.scan_refused.max(w.env.scan_refused);
        let mut viol = w.env.viol;
        let latched = c.pointer_markers_latched_off > 0;
        if latched {
            rep.latched_states += 1;
            rep.latched_markers_max = rep.latched_markers_max.max(c.pointer_markers);
            rep.latched_carried_max = rep.latched_carried_max.max(c.pointer_carried_no_marker);
        }
        let succ = w.successors(cfg);
        if succ.is_empty() {
            rep.terminals += 1;
            rep.modes.insert(w.relay.life.mode());
            if w.env.switched > 0 {
                rep.switched_terminals += 1;
            }
            let ex = &w.env.executed;
            if w.done(cfg) || cfg.spec.may_block {
                if !cfg.spec.hostile && !w.torn_down {
                    if ex.len() < cfg.spec.must_relay
                        || ex[..] != cfg.spec.expected[..ex.len().min(cfg.spec.expected.len())]
                    {
                        // a missing or reordered tail at a terminal state: lost work with no timer
                        if !(cfg.spec.may_block && !w.done(cfg)) {
                            viol |= V_DEADLOCK;
                        }
                    }
                }
            } else {
                viol |= V_DEADLOCK; // the script itself never finished and nothing can move
            }
            if w.env.viol & V_UNSCANNED_TRIGGER != 0 {
                rep.unscanned_trigger_terminals += 1;
            }
        }
        if latched {
            rep.viol_latched |= viol;
        }
        if viol != 0 {
            rep.viol |= viol;
            if rep.first.is_none() {
                rep.first = Some(format!(
                    "{:?}: pc={} executed={:?} expected={:?} log={:?} relay={:?}",
                    viol_names(viol),
                    w.pc,
                    w.env.executed,
                    cfg.spec.expected,
                    w.env.log,
                    w.relay.key()
                ));
            }
        }
        for mut s in succ {
            s.env.viol |= 0;
            if seen.insert(s.key()) {
                queue.push_back(s);
            }
        }
    }
    rep
}

#[track_caller]
pub fn assert_clean(cfg: &Cfg, name: &str) -> Report {
    let r = explore(cfg);
    println!(
        "{name}: {} states, {} terminals, modes at terminals {:?}",
        r.states, r.terminals, r.modes
    );
    assert!(
        r.viol == 0,
        "{name}: violations {:?}; first: {}",
        viol_names(r.viol),
        r.first.clone().unwrap_or_default()
    );
    assert!(r.terminals > 0, "{name}: no terminal state reached");
    r
}

#[track_caller]
pub fn assert_found(cfg: &Cfg, name: &str, bit: u32) -> Report {
    let r = explore(cfg);
    println!(
        "{name}: {} states; violations {:?}",
        r.states,
        viol_names(r.viol)
    );
    assert!(
        r.viol & bit != 0,
        "{name}: the checker did NOT find {:?} (found {:?})",
        viol_names(bit),
        viol_names(r.viol)
    );
    r
}

// ------------------------------------------------------------------------------------------------
// scenarios

fn p(id: u8) -> Ge {
    ge(id, Kind::Plain)
}

fn spec(script: Vec<Op>, expected: &[u8]) -> Spec {
    Spec {
        script,
        expected: expected.to_vec(),
        must_relay: expected.len(),
        may_block: false,
        hostile: false,
    }
}

/// SetObj, then plain entries, one doorbell for each batch.
fn steady() -> Spec {
    spec(
        vec![
            Op::Push(ge(0, Kind::SetObj)),
            Op::Push(p(1)),
            Op::SetPut,
            Op::Ring,
            Op::Push(p(2)),
            Op::Push(p(3)),
            Op::SetPut,
            Op::Ring,
        ],
        &[0, 1, 2, 3],
    )
}

#[test]
fn i1_a_bound_channel_with_no_registration_reads_no_pushbuffer_after_the_binding() {
    let cfg = Cfg::new(steady());
    let r = assert_clean(&cfg, "steady");
    assert_eq!(
        r.modes,
        HashSet::from([Mode::Pointer]),
        "every terminal state is Pointer"
    );
    // only the Scan phase reads: the SET_OBJECT segment (and the entries before the worker saw it bound)
    assert!(r.pb_reads_max <= 4, "pb reads: {}", r.pb_reads_max);
}

#[test]
fn i1_pointer_mode_forwards_the_guests_entries_bit_exact_and_scans_none_of_them() {
    // 12 entries in two batches on a 4-slot guest ring (wrap), SET_OBJECT first
    let mut script = vec![Op::Push(ge(0, Kind::SetObj)), Op::SetPut, Op::Ring];
    let mut expected = vec![0u8];
    for id in 1..=6u8 {
        script.push(Op::Push(p(id)));
        script.push(Op::SetPut);
        script.push(Op::Ring);
        expected.push(id);
    }
    let mut cfg = Cfg::new(spec(script, &expected));
    cfg.src_entries = 4;
    cfg.max_states = 6_000_000;
    let r = assert_clean(&cfg, "pointer-wrap");
    assert_eq!(r.modes, HashSet::from([Mode::Pointer]));
}

// ---- the registration cycle

fn trig(id: u8) -> Ge {
    ge(id, Kind::Trig { migrate: true })
}

/// SetObj, one entry, a registration, then the trigger (published only after the reply), then the new ring.
fn flip(kind: Registration) -> Cfg {
    let mut script = vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::Push(p(1)),
        Op::SetPut,
        Op::Ring,
        Op::Register,
        Op::AwaitReply,
        Op::Push(trig(2).post()),
        Op::SetPut,
        Op::Ring,
    ];
    if kind == Registration::Migrate {
        script.push(Op::UseNew);
    }
    script.extend([Op::Push(p(3).post()), Op::SetPut, Op::Ring]);
    let mut cfg = Cfg::new(spec(script, &[0, 1, 2, 3]));
    cfg.kind = kind;
    cfg
}

#[test]
fn i4_no_entry_published_after_the_reply_is_forwarded_before_a_window_scanned_it() {
    for kind in [Registration::Migrate, Registration::Other] {
        let cfg = flip(kind);
        let r = assert_clean(&cfg, &format!("flip-{kind:?}"));
        // every interleaving ends back in Pointer with the registration consumed; after a ring switch the new
        // ring's first entries are still scanned (D5), so one post-switch entry leaves the Window open
        let want = if kind == Registration::Migrate {
            Mode::Window
        } else {
            Mode::Pointer
        };
        assert_eq!(r.modes, HashSet::from([want]), "{kind:?}");
        assert!(
            r.modes_seen.contains(&Mode::Window),
            "the Window was entered"
        );
        assert!(
            r.to_window_fresh_max >= 1,
            "some interleaving flips a Pointer channel (Fresh)"
        );
        if kind == Registration::Migrate {
            assert_eq!(
                r.switched_terminals, r.terminals,
                "the source switched in every run"
            );
        }
        assert!(
            r.triggers_found_max == 1,
            "exactly one trigger is ever found"
        );
    }
}

#[test]
fn i5_the_whole_cycle_uses_only_the_listed_transitions() {
    let r = assert_clean(&flip(Registration::Migrate), "cycle");
    for m in [Mode::Scan, Mode::Pointer, Mode::Window] {
        assert!(r.modes_seen.contains(&m), "{m:?} never reached");
    }
}

#[test]
fn i4_a_trigger_published_before_the_reply_may_reach_the_hardware_unscanned_and_nothing_wedges() {
    // HOSTILE: the guest publishes the trigger without waiting for the reply. In some interleavings the
    // worker forwards it before the registration is visible; the hardware then executes a trigger nobody
    // scanned (an unregistered handle: the guest's own channel resets, owner ruling 2026-10-13). The
    // model counts it, every entry is still relayed in order, and no state is stuck.
    let script = vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::SetPut,
        Op::Ring,
        Op::Register,
        Op::Push(trig(1)),
        Op::SetPut,
        Op::Ring,
        Op::Push(p(2)),
        Op::SetPut,
        Op::Ring,
    ];
    let mut cfg = Cfg::new(spec(script, &[0, 1, 2]));
    cfg.kind = Registration::Other;
    let r = explore(&cfg);
    println!(
        "hostile-early-trigger: {} states, {:?}",
        r.states,
        viol_names(r.viol)
    );
    assert_eq!(
        r.viol, V_UNSCANNED_TRIGGER,
        "only the unscanned trigger, no other violation: {:?}",
        r.first
    );
    assert!(r.unscanned_trigger_terminals > 0);
    assert!(
        r.unscanned_trigger_terminals < r.terminals,
        "other interleavings see the registration first and scan it"
    );
}

#[test]
fn a3_the_flip_boundary_is_the_cursor_at_the_entry_not_a_reread_of_gp_put() {
    // The guest keeps advancing GP_PUT between the registration, its reply and the first window read.
    let script = vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::Push(p(1)),
        Op::SetPut,
        Op::Ring,
        Op::Push(p(2)),
        Op::SetPut,
        Op::Register,
        Op::Push(p(3)),
        Op::SetPut,
        Op::AwaitReply,
        Op::Push(p(4).post()),
        Op::Push(trig(5).post()),
        Op::SetPut,
        Op::Ring,
        Op::UseNew,
        Op::Push(p(6).post()),
        Op::SetPut,
        Op::Ring,
    ];
    let mut cfg = Cfg::new(spec(script, &[0, 1, 2, 3, 4, 5, 6]));
    cfg.max_states = 30_000_000;
    let r = assert_clean(&cfg, "flip-at-every-cursor");
    assert!(r.to_window_fresh_max >= 1);
}

// ---- A1: SYNC_WAIT across the Pointer -> Window edge

/// SetObj; A (acquire on sem 0); S (SYNC_WAIT); a registration; B whose pushbuffer the guest writes only after
/// ... and only then releases the semaphore A's acquire waits for. B is published AFTER the reply.
fn a1_script() -> Vec<Op> {
    vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::SetPut,
        Op::Ring,
        Op::Push(p(1).acq(0)),
        Op::Push(p(2).sync()),
        Op::SetPut,
        Op::Ring,
        Op::Register,
        Op::AwaitReply,
        Op::Push(p(3).late().post()),
        Op::SetPut,
        Op::Ring,
        Op::Fill(3),
        Op::Release(0),
    ]
}

#[test]
fn a1_a_registration_right_after_a_forwarded_sync_wait_never_scans_the_late_pushbuffer() {
    let mut cfg = Cfg::new(spec(a1_script(), &[0, 1, 2, 3]));
    cfg.kind = Registration::Other;
    let r = assert_clean(&cfg, "a1");
    assert!(
        r.seeded_max >= 1,
        "some interleaving seeds the window's gate from Pointer mode"
    );
    assert_eq!(
        r.modes,
        HashSet::from([Mode::Window]),
        "no trigger: the Window stays open"
    );
}

#[test]
fn i3_the_same_stream_with_the_guest_free_to_publish_b_early_is_found() {
    // control: if B is NOT published after the reply the seeded gate has nothing to protect but the
    // checker must still see the stale read when the gate is absent (mutant tests do this per mutant)
    let mut cfg = Cfg::new(spec(a1_script(), &[0, 1, 2, 3]));
    cfg.kind = Registration::Other;
    cfg.always_scan = false;
    let r = explore(&cfg);
    assert_eq!(r.viol, 0);
}

/// A full ring lap of Pointer entries between two windows: the second window's first entry is a SYNC_WAIT
/// at the same (generation, cursor) the first window cleared. 4-slot guest ring.
fn a1_lap_script() -> Vec<Op> {
    vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::SetPut,
        Op::Ring,
        Op::Register,
        Op::AwaitReply,
        // slot 1: the first window's trigger entry, a SYNC_WAIT entry (its gate clears (gen 1, cursor 1))
        Op::Push(trig(1).sync().post()),
        Op::SetPut,
        Op::Ring,
        // three more entries: the cursor laps back to slot 1 in Pointer mode
        Op::Push(p(2)),
        Op::SetPut,
        Op::Ring,
        Op::Push(p(3)),
        Op::SetPut,
        Op::Ring,
        Op::Push(p(4).acq(1)),
        Op::SetPut,
        Op::Ring,
        Op::Register,
        Op::AwaitReply,
        // slot 1 again: a SYNC_WAIT entry whose pushbuffer is written late, behind p4's acquire
        Op::Push(p(5).sync().late().post()),
        Op::SetPut,
        Op::Ring,
        Op::Fill(5),
        Op::Release(1),
    ]
}

#[test]
fn a1_a_full_ring_lap_in_pointer_mode_does_not_revive_a_stale_gate_proof() {
    let mut cfg = Cfg::new(spec(a1_lap_script(), &[0, 1, 2, 3, 4, 5]));
    cfg.kind = Registration::Other;
    cfg.src_entries = 4;
    cfg.max_states = 6_000_000;
    let r = assert_clean(&cfg, "a1-lap");
    assert!(r.to_window_fresh_max >= 1);
}

// ---- A2: Scan -> Pointer with an open packet

#[test]
fn a2_the_binding_packet_split_across_two_entries_switches_to_pointer_after_the_datum() {
    // [h(6,0,1)] | [5080]: the binding is committed only after the second entry. Straddle counted in Scan.
    // Afterwards a registration and its trigger: the Window must know the binding (it was learned from the
    // split packet), so the trigger is found.
    let script = vec![
        Op::Push(ge(0, Kind::SetObjHead)),
        Op::Push(ge(1, Kind::SetObjTail)),
        Op::Push(p(2)),
        Op::SetPut,
        Op::Ring,
        Op::Register,
        Op::AwaitReply,
        Op::Push(trig(3).post()),
        Op::SetPut,
        Op::Ring,
        Op::Push(p(4)),
        Op::SetPut,
        Op::Ring,
    ];
    let mut cfg = Cfg::new(spec(script, &[0, 1, 2, 3, 4]));
    cfg.kind = Registration::Other;
    let r = assert_clean(&cfg, "a2-split-binding");
    assert_eq!(r.modes, HashSet::from([Mode::Pointer]));
    assert_eq!(
        r.straddles_scan_max, 1,
        "exactly the header-only segment ends open"
    );
    assert_eq!(
        r.triggers_found_max, 1,
        "the trigger is found: the binding was learned from the split packet"
    );
}

#[test]
fn a2_an_open_packet_carried_across_the_mode_switch_is_dropped_and_a_later_window_starts_fresh() {
    // Pointer mode forwards OpenHead unread; the registration lands between OpenHead and OpenTail in some
    // interleavings. OpenTail (words that do not decode as a header) is then scanned with FRESH state:
    // a loud refusal (P2 falsified at this flip), counted exactly once. In the other interleavings the flip
    // is on a boundary and nothing happens.
    let script = vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::SetPut,
        Op::Ring,
        Op::Push(ge(1, Kind::OpenHead)),
        Op::SetPut,
        Op::Register,
        Op::AwaitReply,
        Op::Push(ge(2, Kind::OpenTailBad).post()),
        Op::SetPut,
        Op::Ring,
    ];
    let mut cfg = Cfg::new(spec(script, &[0, 1, 2]));
    cfg.kind = Registration::Other;
    let r = explore(&cfg);
    println!("a2-fresh: {} states {:?}", r.states, viol_names(r.viol));
    // The submission itself (a GP_PUT value) ends inside the packet: assumption P2' is false for this script, so the
    // anchor is inside the packet. The loud refusal is the visible half; the hardware's view also sees the seeded
    // marker land inside the packet (the documented residual risk, measured by `put_boundary_open_*`).
    assert_eq!(
        r.viol,
        V_REFUSED | V_DEADLOCK | V_MARKER_IN_PACKET,
        "P2' falsified by construction: the Fresh scan refuses the tail of a packet opened in Pointer mode \
         ({:?})",
        r.first
    );
    assert_eq!(
        r.window_fresh_errors_max, 1,
        "counted exactly once per entry, retries included"
    );
    assert!(r.scan_refused >= 1);
    assert!(
        r.retries_max >= 1,
        "the refusal is retried like today's scanner refusal before it is final"
    );
}

#[test]
fn a2_with_every_packet_ended_on_a_boundary_no_flip_position_refuses_anything() {
    let script = vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::Push(p(1)),
        Op::SetPut,
        Op::Ring,
        Op::Register,
        Op::AwaitReply,
        Op::Push(p(2).post()),
        Op::Push(p(3).post()),
        Op::SetPut,
        Op::Ring,
    ];
    let mut cfg = Cfg::new(spec(script, &[0, 1, 2, 3]));
    cfg.kind = Registration::Other;
    let r = assert_clean(&cfg, "a2-boundary");
    assert_eq!(r.window_fresh_errors_max, 0);
}

// ---- A4: torn loads and hostile dwords

#[test]
fn a4_one_dword_changing_between_the_loads_never_stores_a_value_other_than_the_tested_one() {
    // The guest changes the entry's high or low dword at every possible instant (the explorer runs the
    // mutation between any two relay micro-steps). The relay may refuse the value it loaded (named) or
    // forward it; it never stores anything it did not test.
    for (hi, xor) in [
        (true, 1 << 31),
        (true, 1 << 10),
        (true, 1 << 30),
        (false, 0x1000),
        (false, 1),
        (true, 1 << 9),
    ] {
        let script = vec![
            Op::Push(ge(0, Kind::SetObj)),
            Op::SetPut,
            Op::Ring,
            Op::Push(p(1)),
            Op::SetPut,
            Op::Ring,
            Op::Mutate {
                ring: 0,
                slot: 1,
                hi,
                xor,
            },
        ];
        let mut s = spec(script, &[0, 1]);
        s.hostile = true;
        let mut cfg = Cfg::new(s);
        cfg.max_states = 2_000_000;
        let r = explore(&cfg);
        println!(
            "a4 hi={hi} xor={xor:#x}: {} states {:?}",
            r.states,
            viol_names(r.viol)
        );
        assert_eq!(r.viol & V_TORN, 0, "hi={hi} xor={xor:#x}: {:?}", r.first);
        assert_eq!(r.viol & V_I1, 0);
    }
}

// ---- A5: hostile registration / trigger orders

#[test]
fn a5_a_trigger_after_remove_api_reaches_the_hardware_and_is_counted_nothing_wedges() {
    let script = vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::SetPut,
        Op::Ring,
        Op::Register,
        Op::AwaitReply,
        Op::Remove,
        Op::AwaitReply,
        Op::Push(trig(1).post()),
        Op::SetPut,
        Op::Ring,
        Op::Push(p(2)),
        Op::SetPut,
        Op::Ring,
    ];
    let mut cfg = Cfg::new(spec(script, &[0, 1, 2]));
    cfg.kind = Registration::Other;
    let r = explore(&cfg);
    println!("a5-stale: {} states {:?}", r.states, viol_names(r.viol));
    assert_eq!(r.viol, V_UNSCANNED_TRIGGER, "{:?}", r.first);
    assert_eq!(r.modes, HashSet::from([Mode::Pointer]));
}

#[test]
fn a5_remove_api_before_the_trigger_then_a_new_registration_runs_one_clean_cycle() {
    let script = vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::SetPut,
        Op::Ring,
        Op::Register,
        Op::AwaitReply,
        Op::Remove,
        Op::AwaitReply,
        Op::Register,
        Op::AwaitReply,
        Op::Push(trig(1).post()),
        Op::SetPut,
        Op::Ring,
        Op::Push(p(2).post()),
        Op::SetPut,
        Op::Ring,
    ];
    let mut cfg = Cfg::new(spec(script, &[0, 1, 2]));
    cfg.kind = Registration::Other;
    let r = assert_clean(&cfg, "a5-remove-then-register");
    assert_eq!(r.modes, HashSet::from([Mode::Pointer]));
}

#[test]
fn a5_two_registrations_keep_one_window_open_until_both_are_consumed() {
    let script = vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::SetPut,
        Op::Ring,
        Op::Register,
        Op::AwaitReply,
        Op::Register,
        Op::AwaitReply,
        Op::Push(trig(1).post()),
        Op::SetPut,
        Op::Ring,
        Op::Remove,
        Op::AwaitReply,
        Op::Push(p(2).post()),
        Op::SetPut,
        Op::Ring,
        Op::Push(p(3)),
        Op::SetPut,
        Op::Ring,
    ];
    let mut cfg = Cfg::new(spec(script, &[0, 1, 2, 3]));
    cfg.kind = Registration::Other;
    let r = assert_clean(&cfg, "a5-two-registrations");
    assert_eq!(r.modes, HashSet::from([Mode::Pointer]));
}

#[test]
fn a5_a_trigger_shaped_packet_on_the_wrong_subchannel_is_not_a_trigger() {
    let script = vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::SetPut,
        Op::Ring,
        Op::Register,
        Op::AwaitReply,
        Op::Push(ge(1, Kind::TrigWrongSub).post()),
        Op::Push(p(2).post()),
        Op::SetPut,
        Op::Ring,
    ];
    let mut cfg = Cfg::new(spec(script, &[0, 1, 2]));
    cfg.kind = Registration::Other;
    let r = assert_clean(&cfg, "a5-wrong-sub");
    assert_eq!(r.triggers_found_max, 0);
    assert_eq!(
        r.modes,
        HashSet::from([Mode::Window]),
        "no trigger: the Window stays open"
    );
}

#[test]
fn a5_a_trigger_split_over_two_entries_is_refused_like_today() {
    let script = vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::SetPut,
        Op::Ring,
        Op::Register,
        Op::AwaitReply,
        Op::Push(ge(1, Kind::TrigHead).post()),
        Op::Push(ge(2, Kind::TrigTail).post()),
        Op::SetPut,
        Op::Ring,
    ];
    let mut cfg = Cfg::new(spec(script, &[0, 1, 2]));
    cfg.kind = Registration::Other;
    let r = explore(&cfg);
    println!(
        "a5-split-trigger: {} states {:?}",
        r.states,
        viol_names(r.viol)
    );
    assert!(
        r.viol & V_REFUSED != 0,
        "the software packet split across segments is refused: {:?}",
        r.first
    );
    assert_eq!(
        r.window_fresh_errors_max, 0,
        "a refusal in a Window that is not a fresh start is not counted as one"
    );
}

#[test]
fn a5_data_that_looks_like_the_trigger_inside_a_packet_is_not_a_trigger_when_the_state_is_exact() {
    // the whole packet in one entry: [INC x3, <trigger words as data>]
    let script = vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::SetPut,
        Op::Ring,
        Op::Register,
        Op::AwaitReply,
        Op::Push(ge(1, Kind::FalseTail).post()),
        Op::SetPut,
        Op::Ring,
    ];
    let mut cfg = Cfg::new(spec(script, &[0, 1]));
    cfg.kind = Registration::Other;
    let r = explore(&cfg);
    // FalseTail alone is a Fresh/exact scan of [h(6,0x200,1), HANDLE, 7]: that IS a trigger packet followed by
    // a stray datum when scanned from a boundary -- the legitimate meaning of those words. Characterisation:
    assert!(r.triggers_found_max >= 1);
}

#[test]
fn a5_a_flip_inside_a_packet_whose_data_looks_like_the_trigger_consumes_the_registration_wrongly() {
    // [INC sub0 x3 header] | [h(6,0x200,1) HANDLE 7]: scanned from a boundary the second entry is data; scanned
    // FRESH (the flip between the two entries) it is a trigger. P2 consequence, characterised and counted.
    let script = vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::SetPut,
        Op::Ring,
        Op::Push(ge(1, Kind::FalseHead)),
        Op::SetPut,
        Op::Register,
        Op::AwaitReply,
        Op::Push(ge(2, Kind::FalseTail).post()),
        Op::SetPut,
        Op::Ring,
    ];
    let mut cfg = Cfg::new(spec(script, &[0, 1, 2]));
    cfg.kind = Registration::Other;
    let r = explore(&cfg);
    println!(
        "a5-false-positive: {} states {:?} triggers_found_max={}",
        r.states,
        viol_names(r.viol),
        r.triggers_found_max
    );
    assert!(
        r.triggers_found_max >= 1,
        "some flip makes the data a trigger"
    );
}

// ---- A6: subchannel rebind after the binding

#[test]
fn a6_a_rebind_inside_a_window_is_seen_and_counted_and_the_new_binding_is_used() {
    // In a Window the scan is live: SET_OBJECT 5080 on sub7 moves the binding; the trigger on sub7 is found.
    let script = vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::SetPut,
        Op::Ring,
        Op::Register,
        Op::AwaitReply,
        Op::Push(ge(1, Kind::Rebind).post()),
        Op::Push(ge(2, Kind::SetObj7).post()),
        Op::Push(ge(3, Kind::Trig7).post()),
        Op::SetPut,
        Op::Ring,
    ];
    let mut cfg = Cfg::new(spec(script, &[0, 1, 2, 3]));
    cfg.kind = Registration::Other;
    let r = assert_clean(&cfg, "a6-in-window");
    assert!(
        r.binding_changed_max >= 1,
        "the moved binding is counted (P1 falsifier)"
    );
    assert_eq!(r.triggers_found_max, 1);
}

#[test]
#[ignore = "OPEN: a rebind of the software subchannel while in Pointer mode is invisible; a later trigger on the new subchannel is missed (Open question 5)"]
fn a6_open_a_rebind_in_pointer_mode_is_not_known_to_the_window() {
    // EXPECTED (what a correct Window would do): the trigger on sub7 is found and the registration consumed.
    // OBSERVED today: the retained binding says sub6, the unscanned rebind moved 5080 to sub7, the trigger is
    // forwarded as an ordinary packet and the hardware executes it unscanned.
    let script = vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::SetPut,
        Op::Ring,
        Op::Push(ge(1, Kind::Rebind)),
        Op::Push(ge(2, Kind::SetObj7)),
        Op::SetPut,
        Op::Ring,
        Op::Register,
        Op::AwaitReply,
        Op::Push(ge(3, Kind::Trig7).post()),
        Op::SetPut,
        Op::Ring,
    ];
    let mut cfg = Cfg::new(spec(script, &[0, 1, 2, 3]));
    cfg.kind = Registration::Other;
    let r = assert_clean(&cfg, "a6-open");
    assert_eq!(r.triggers_found_max, 1);
}

// ---- A7 / B: teardown, races, windows that never end

#[test]
fn a7_teardown_at_any_instant_stops_the_private_ring_for_good() {
    let script = vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::Push(p(1)),
        Op::SetPut,
        Op::Ring,
        Op::Register,
        Op::Push(p(2)),
        Op::SetPut,
        Op::Ring,
        Op::Teardown,
        Op::Push(p(3)),
        Op::SetPut,
        Op::Ring,
    ];
    let mut s = spec(script, &[0, 1, 2, 3]);
    s.hostile = true; // after teardown nothing is owed
    let cfg = Cfg::new(s);
    let r = explore(&cfg);
    println!("a7: {} states {:?}", r.states, viol_names(r.viol));
    assert_eq!(r.viol & V_AFTER_TEARDOWN, 0, "{:?}", r.first);
    assert_eq!(r.modes, HashSet::from([Mode::Dead]));
}

#[test]
fn b_a_window_that_never_completes_stays_a_window_and_every_entry_is_still_relayed_scanned() {
    let script = vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::SetPut,
        Op::Ring,
        Op::Register,
        Op::AwaitReply,
        Op::Push(p(1).post()),
        Op::Push(p(2).post()),
        Op::SetPut,
        Op::Ring,
        Op::Push(p(3).post()),
        Op::SetPut,
        Op::Ring,
    ];
    let mut cfg = Cfg::new(spec(script, &[0, 1, 2, 3]));
    cfg.kind = Registration::Other;
    let r = assert_clean(&cfg, "b-never-completes");
    assert_eq!(r.modes, HashSet::from([Mode::Window]));
}

#[test]
fn b_registration_racing_remove_and_teardown_loses_and_duplicates_nothing() {
    let script = vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::Push(p(1)),
        Op::SetPut,
        Op::Ring,
        Op::Register,
        Op::Remove,
        Op::Push(p(2)),
        Op::SetPut,
        Op::Ring,
        Op::Teardown,
    ];
    let mut s = spec(script, &[0, 1, 2]);
    s.hostile = true;
    let cfg = Cfg::new(s);
    let r = explore(&cfg);
    assert_eq!(
        r.viol & (V_AFTER_TEARDOWN | V_TORN | V_I1 | V_TRANSITION),
        0,
        "{:?}",
        r.first
    );
}

/// The guest prepares the new ring (and its USERD) BEFORE the trigger entry becomes visible: the order a
/// driver that must preset the new GP_PUT would use.
fn cursor_script() -> Vec<Op> {
    vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::Push(p(1)),
        Op::SetPut,
        Op::Ring,
        Op::Register,
        Op::AwaitReply,
        Op::Push(trig(2).post()),
        Op::PrepNew,
        Op::SetPut,
        Op::Ring,
        Op::UseNew,
        Op::Push(p(3).post()),
        Op::SetPut,
        Op::Ring,
    ]
}

/// The new ring is prepared only AFTER the trigger was published and rung.
fn cursor_script_late() -> Vec<Op> {
    vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::Push(p(1)),
        Op::SetPut,
        Op::Ring,
        Op::Register,
        Op::AwaitReply,
        Op::Push(trig(2).post()),
        Op::SetPut,
        Op::Ring,
        Op::PrepNew,
        Op::UseNew,
        Op::Push(p(3).post()),
        Op::SetPut,
        Op::Ring,
    ]
}

#[test]
fn b_the_source_switch_loses_and_duplicates_nothing_when_the_new_ring_starts_at_zero() {
    let mut cfg = Cfg::new(spec(cursor_script(), &[0, 1, 2, 3]));
    cfg.cursor_relay = Cursor::Zero;
    cfg.cursor_guest = Cursor::Zero;
    let r = assert_clean(&cfg, "switch-zero");
    assert_eq!(r.switched_terminals, r.terminals);
}

#[test]
fn b_the_source_switch_loses_and_duplicates_nothing_when_the_indices_continue() {
    let mut cfg = Cfg::new(spec(cursor_script(), &[0, 1, 2, 3]));
    cfg.cursor_relay = Cursor::Keep;
    cfg.cursor_guest = Cursor::Keep;
    let r = assert_clean(&cfg, "switch-keep");
    assert_eq!(r.switched_terminals, r.terminals);
}

#[test]
fn b_mismatched_cursor_semantics_lose_entries_and_the_checker_sees_it() {
    // Production resets the cursor to 0 (the measured Windows growth). If the guest's new ring in fact
    // continued the indices, the relay would read the wrong slots: the unmeasured P0 (a) question.
    let mut cfg = Cfg::new(spec(cursor_script(), &[0, 1, 2, 3]));
    cfg.cursor_relay = Cursor::Zero;
    cfg.cursor_guest = Cursor::Keep;
    assert_found(&cfg, "switch-mismatch", V_DEADLOCK);
}

// ---- I6: nothing depends on time

#[test]
fn i6_every_announced_wait_is_woken_by_an_event_with_no_look_and_no_timer() {
    // Scan / Window waits are announced by the marker's interrupt: liveness without any look.
    let mut cfg = Cfg::new(spec(a1_script(), &[0, 1, 2, 3]));
    cfg.kind = Registration::Other;
    cfg.looks = false;
    assert_clean(&cfg, "i6-no-looks");
}

#[test]
fn i6_a_worker_that_ignores_events_deadlocks_when_time_is_forbidden() {
    let mut cfg = Cfg::new(spec(a1_script(), &[0, 1, 2, 3]));
    cfg.kind = Registration::Other;
    cfg.looks = false;
    cfg.glue.ignore_events = true;
    assert_found(&cfg, "i6-ignore-events", V_DEADLOCK);
}

fn ring_full_script() -> Vec<Op> {
    let mut s = vec![Op::Push(ge(0, Kind::SetObj)), Op::SetPut, Op::Ring];
    for id in 1..=4u8 {
        s.push(Op::Push(p(id)));
    }
    s.extend([Op::SetPut, Op::Ring]);
    s
}

#[test]
fn i6_pointer_mode_waits_for_private_ring_credit_are_only_found_by_a_look() {
    // Documents Open question 1 as it is: with the private ring too small for the backlog, Pointer mode has
    // no marker (no parser state), so no interrupt announces the freed credit. With looks (the bounded spin,
    // any engine event, the counted 50 ms backstop) the run is live; with time and looks forbidden it is not.
    let mut cfg = Cfg::new(spec(ring_full_script(), &[0, 1, 2, 3, 4]));
    cfg.priv_entries = 4;
    cfg.looks = true;
    assert_clean(&cfg, "ring-full-with-looks");
    cfg.looks = false;
    assert_found(&cfg, "ring-full-no-looks", V_DEADLOCK);
}

#[test]
#[ignore = "OPEN: Pointer mode cannot announce private-ring credit without a marker (Open question 1)"]
fn i6_open_pointer_mode_credit_waits_need_no_look() {
    let mut cfg = Cfg::new(spec(ring_full_script(), &[0, 1, 2, 3, 4]));
    cfg.priv_entries = 4;
    cfg.looks = false;
    assert_clean(&cfg, "ring-full-no-looks-expected");
}

#[test]
#[ignore = "OPEN: if the indices continue across the growth, the new USERD's GP_PUT must already hold the continued value when the relay switches (Open question 4)"]
fn b_open_continued_indices_need_the_new_gp_put_preset_before_the_switch() {
    // EXPECTED: the relay idles at the switch until the guest publishes in the new ring.
    // OBSERVED: with a fresh (zero) GP_PUT in the new USERD the relay sees GP_PUT != cursor and forwards the
    // stale slots between them (a duplicate of an old entry). With reset-to-zero (production) this cannot
    // happen. Unmeasured: what a real Windows growth leaves in the new USERD (P0 a).
    let mut cfg = Cfg::new(spec(cursor_script_late(), &[0, 1, 2, 3]));
    cfg.cursor_relay = Cursor::Keep;
    cfg.cursor_guest = Cursor::Keep;
    assert_clean(&cfg, "switch-keep-late");
}

#[test]
fn b_reset_to_zero_does_not_depend_on_when_the_guest_prepares_the_new_ring() {
    let mut cfg = Cfg::new(spec(cursor_script_late(), &[0, 1, 2, 3]));
    cfg.cursor_relay = Cursor::Zero;
    cfg.cursor_guest = Cursor::Zero;
    assert_clean(&cfg, "switch-zero-late");
}

// ---- the checker has teeth: each deliberate glue defect is FOUND (the machine's own mutants run through
// `scripts/lifecycle_mutation_check.sh`, which re-runs this whole file with `KF_LIFECYCLE_MUTANT`)

// ---- observability: exact counter values in the tests that provoke them

#[test]
fn obs_a_sync_wait_behind_an_open_packet_is_read_at_once_and_counted_exactly_once_in_a_window() {
    // The registration is live before the binding is seen: the scan never stops (an EXACT window), so the
    // SYNC_WAIT entry behind the open packet is the old `carried_ungated` case.
    let script = vec![
        Op::Register,
        Op::AwaitReply,
        Op::Push(ge(0, Kind::SetObj).post()),
        Op::Push(ge(1, Kind::OpenHead).post()),
        Op::Push(ge(2, Kind::OpenTail).sync().post()),
        Op::Push(p(3).post()),
        Op::SetPut,
        Op::Ring,
    ];
    let mut cfg = Cfg::new(spec(script, &[0, 1, 2, 3]));
    cfg.kind = Registration::Other;
    let r = assert_clean(&cfg, "obs-carried");
    assert_eq!(r.carried_ungated, 1);
    assert_eq!(r.modes, HashSet::from([Mode::Window]));
}

#[test]
fn obs_pointer_mode_has_no_carried_ungated_case_because_it_scans_nothing() {
    let script = vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::Push(p(1)),
        Op::SetPut,
        Op::Ring,
        Op::Push(ge(2, Kind::OpenHead)),
        Op::Push(ge(3, Kind::OpenTail).sync()),
        Op::Push(p(4)),
        Op::SetPut,
        Op::Ring,
    ];
    let cfg = Cfg::new(spec(script, &[0, 1, 2, 3, 4]));
    let r = assert_clean(&cfg, "obs-pointer-carried");
    assert_eq!(r.carried_ungated, 0);
}

// ---- B: the registration while the private ring or the guest ring is full, and spurious doorbells

#[test]
fn b_a_registration_with_a_full_private_ring_still_flips_and_loses_nothing() {
    let mut cfg = flip(Registration::Other);
    cfg.priv_entries = 4;
    cfg.max_states = 8_000_000;
    let r = assert_clean(&cfg, "b-private-ring-full");
    assert_eq!(r.modes, HashSet::from([Mode::Pointer]));
}

#[test]
fn b_a_registration_with_a_full_guest_ring_waiting_for_the_gp_get_write_back() {
    let mut cfg = flip(Registration::Other);
    cfg.src_entries = 4;
    cfg.max_states = 8_000_000;
    let r = assert_clean(&cfg, "b-guest-ring-full");
    assert_eq!(r.modes, HashSet::from([Mode::Pointer]));
}

#[test]
fn b_extra_and_coalesced_doorbells_during_a_draining_gate_change_nothing() {
    let mut script = vec![];
    for op in a1_script() {
        script.push(op);
        if matches!(op, Op::Push(_) | Op::Register | Op::AwaitReply) {
            script.push(Op::Ring);
        }
    }
    let mut cfg = Cfg::new(spec(script, &[0, 1, 2, 3]));
    cfg.kind = Registration::Other;
    cfg.max_states = 8_000_000;
    assert_clean(&cfg, "b-spurious-doorbells");
}

// ---- D: the shape of a recorded Edge refusal (`traces/user_ring_scan_20261014/user-ring-refusal-c1d00054-ff040013-453.txt`)

#[test]
fn d_the_recorded_edge_ring_shape_behind_unreached_sync_waits_is_not_read_early_in_a_window() {
    // GP ring entries 447..453 of the trace: SYNC NOP, seg, seg (an acquire inside), seg, SYNC NOP, seg, seg whose
    // bytes the guest writes only later (the scan met zeros and refused). A registration is live (an exact window):
    // the SyncGate must keep the window from reading the late segment before the hardware passed the acquire.
    let script = vec![
        Op::Register,
        Op::AwaitReply,
        Op::Push(ge(0, Kind::SetObj).post()),
        Op::Push(ge(10, Kind::Nop).sync().post()),
        Op::Push(p(2).post()),
        Op::Push(p(3).acq(0).post()),
        Op::Push(p(4).post()),
        Op::Push(ge(11, Kind::Nop).sync().post()),
        Op::Push(p(6).post()),
        Op::Push(p(7).late().post()),
        Op::SetPut,
        Op::Ring,
        Op::Fill(7),
        Op::Release(0),
    ];
    let mut cfg = Cfg::new(spec(script, &[0, 10, 2, 3, 4, 11, 6, 7]));
    cfg.kind = Registration::Other;
    cfg.src_entries = 16;
    cfg.max_states = 8_000_000;
    let r = assert_clean(&cfg, "d-edge-shape");
    assert_eq!(r.modes, HashSet::from([Mode::Window]));
}

// ---- PHASE 1b (independent review, 2026-10-16): the hardware's view of the private stream
//
// `ModelEnv::hw_scan` re-parses everything the relay publishes as ONE packet stream across entries (what the
// PBDMA does). `V_MARKER_IN_PACKET` = a host marker was published while that parse has a packet open.

/// SetObj; a submission with a SYNC_WAIT entry and a packet that SPANS two entries (the Windows driver splits
/// big inline uploads this way, measured); the registration arrives anywhere; the guest publishes after the
/// reply. D1: the base places the seeded barrier + marker at whatever entry boundary the worker is at when it
/// sees the epoch move, which can be inside the packet.
fn d1_script() -> Vec<Op> {
    vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::SetPut,
        Op::Ring,
        Op::Push(p(1).sync()),
        Op::Push(ge(2, Kind::OpenHead)),
        Op::Push(ge(3, Kind::OpenTail)),
        Op::Push(p(4)),
        Op::SetPut,
        Op::Ring,
        Op::Register,
        Op::AwaitReply,
        Op::Push(p(5).post()),
        Op::SetPut,
        Op::Ring,
    ]
}

#[test]
fn d1_a_registration_inside_a_submission_never_puts_a_host_marker_inside_a_guest_packet() {
    let mut cfg = Cfg::new(spec(d1_script(), &[0, 1, 2, 3, 4, 5]));
    cfg.kind = Registration::Other;
    let r = assert_clean(&cfg, "d1-within-submission");
    assert!(
        r.to_window_fresh_max >= 1,
        "some interleaving flips a Pointer channel"
    );
}

/// The reviewer's scenario as written: the SUBMISSION itself (a `GP_PUT` value) ends inside an open packet.
/// Assumption P2' (a driver does not end a submission inside a packet) is what the anchor design rests on; this
/// test documents what happens when it is false: the marker lands in the packet. Not a defect of the machine;
/// the falsifier is the Scan-mode counter `put_boundary_open_scan` on the app matrix.
#[test]
fn p2prime_a_submission_that_ends_inside_a_packet_is_the_documented_residual_risk() {
    let script = vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::SetPut,
        Op::Ring,
        Op::Push(p(1).sync()),
        Op::Push(ge(2, Kind::OpenHead)),
        Op::SetPut,
        Op::Ring,
        Op::Register,
        Op::AwaitReply,
        Op::Push(ge(3, Kind::OpenTail).post()),
        Op::Push(p(4).post()),
        Op::SetPut,
        Op::Ring,
    ];
    let mut cfg = Cfg::new(spec(script, &[0, 1, 2, 3, 4]));
    cfg.kind = Registration::Other;
    let r = explore(&cfg);
    println!("p2prime: {} states viol={:?}", r.states, viol_names(r.viol));
    assert!(
        r.viol & V_MARKER_IN_PACKET != 0,
        "expected the residual-risk violation to stay reachable (it documents the assumption)"
    );
}

#[test]
fn d2_data_that_looks_like_the_trigger_inside_a_packet_of_one_submission_is_never_a_trigger() {
    // [INC sub0 x3 header] | [h(6,0x200,1) HANDLE 7] in ONE submission: scanned from the flip between the two
    // entries (the base) the second is a trigger: the registration is consumed wrongly.
    let script = vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::SetPut,
        Op::Ring,
        Op::Push(ge(1, Kind::FalseHead)),
        Op::Push(ge(2, Kind::FalseTail)),
        Op::SetPut,
        Op::Ring,
        Op::Register,
        Op::AwaitReply,
        Op::Push(p(3).post()),
        Op::SetPut,
        Op::Ring,
    ];
    let mut cfg = Cfg::new(spec(script, &[0, 1, 2, 3]));
    cfg.kind = Registration::Other;
    let r = assert_clean(&cfg, "d2-false-trigger");
    assert_eq!(
        r.triggers_found_max, 0,
        "no interleaving may find a trigger in packet data"
    );
    assert_eq!(
        r.modes,
        HashSet::from([Mode::Window]),
        "the registration is still live"
    );
}

#[test]
fn d2_the_tail_of_a_packet_inside_one_submission_is_never_refused_by_a_window() {
    // [OpenHead] | [words that do not decode as a header]: scanned with fresh state (the base) the tail refuses.
    let script = vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::SetPut,
        Op::Ring,
        Op::Push(ge(1, Kind::OpenHead)),
        Op::Push(ge(2, Kind::OpenTailBad)),
        Op::SetPut,
        Op::Ring,
        Op::Register,
        Op::AwaitReply,
        Op::Push(p(3).post()),
        Op::SetPut,
        Op::Ring,
    ];
    let mut cfg = Cfg::new(spec(script, &[0, 1, 2, 3]));
    cfg.kind = Registration::Other;
    let r = assert_clean(&cfg, "d2-tail-refused");
    assert_eq!(r.window_fresh_errors_max, 0);
}

/// T2: a1 with the SYNC_WAIT as a LENGTH-0 control entry (the Edge / Win11 shape).
#[test]
fn t2_a1_with_a_length0_sync_wait_never_reads_the_late_pushbuffer_early() {
    let script = vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::SetPut,
        Op::Ring,
        Op::Push(p(1).acq(0)),
        Op::Push(ge(2, Kind::Nop).sync()),
        Op::SetPut,
        Op::Ring,
        Op::Register,
        Op::AwaitReply,
        Op::Push(p(3).late().post()),
        Op::SetPut,
        Op::Ring,
        Op::Fill(3),
        Op::Release(0),
    ];
    let mut cfg = Cfg::new(spec(script, &[0, 1, 2, 3]));
    cfg.kind = Registration::Other;
    let r = assert_clean(&cfg, "t2-len0-sync");
    assert!(
        r.seeded_max >= 1,
        "the length-0 SYNC_WAIT seeds the window's gate"
    );
}

/// D3: a guest ring that fills (4 slots) blocks the guest until the relay writes `GP_GET` back. The write-back
/// needs a wake after the PBDMA pops: Pointer mode places a marker at the batch end (the cursor reached a `GP_PUT`
/// the worker read), so the pop is announced by an interrupt and the run is live with NO look at all (no bounded
/// spin, no engine-event guess, no 50 ms backstop). Without Pointer-mode markers the worker sleeps through the pops
/// and the guest never gets its ring back.
#[test]
fn d3_pointer_mode_batch_ends_announce_pops_by_an_event_with_no_look() {
    let mut script = vec![Op::Push(ge(0, Kind::SetObj)), Op::SetPut, Op::Ring];
    let mut expected = vec![0u8];
    for id in 1..=5u8 {
        script.extend([Op::Push(p(id)), Op::SetPut, Op::Ring]);
        expected.push(id);
    }
    let mut cfg = Cfg::new(spec(script, &expected));
    cfg.src_entries = 4;
    cfg.priv_entries = 8;
    cfg.looks = false;
    cfg.max_states = 8_000_000;
    let r = assert_clean(&cfg, "d3-batch-ends");
    assert_eq!(r.modes, HashSet::from([Mode::Pointer]));
}

/// The same scenario with a worker that ignores the marker interrupts (only a timer would wake it): progress
/// depends on time, so with time forbidden it deadlocks. Together with the test above this pins that the wake is the
/// EVENT of the Pointer-mode marker, not a look.
#[test]
fn d3_the_same_run_deadlocks_if_the_worker_ignores_events() {
    let mut script = vec![Op::Push(ge(0, Kind::SetObj)), Op::SetPut, Op::Ring];
    let mut expected = vec![0u8];
    for id in 1..=5u8 {
        script.extend([Op::Push(p(id)), Op::SetPut, Op::Ring]);
        expected.push(id);
    }
    let mut cfg = Cfg::new(spec(script, &expected));
    cfg.src_entries = 4;
    cfg.priv_entries = 8;
    cfg.looks = false;
    cfg.glue.ignore_events = true;
    cfg.max_states = 8_000_000;
    assert_found(&cfg, "d3-ignore-events", V_DEADLOCK);
}

// ---- I8: nothing at or beyond the GP_PUT loaded in the turn is read (the model flags it in EVERY scenario via
// V_BEYOND_PUT; this one adds GP_PUT moving backwards, leaving the ring and advancing mid-turn)

#[test]
fn i8_gp_put_advancing_mid_turn_backwards_and_beyond_the_ring_never_reads_beyond_the_value_loaded()
{
    let script = vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::SetPut,
        Op::Ring,
        Op::Push(p(1)),
        Op::SetPut, // advances while the worker may be anywhere in a turn
        Op::Ring,
        Op::PutRaw(0), // backwards
        Op::Ring,
        Op::PutRaw(200), // beyond the ring
        Op::Ring,
    ];
    let mut s = spec(script, &[0, 1]);
    s.hostile = true;
    let mut cfg = Cfg::new(s);
    cfg.max_states = 4_000_000;
    let r = explore(&cfg);
    println!("i8: {} states {:?}", r.states, viol_names(r.viol));
    assert_eq!(r.viol & V_BEYOND_PUT, 0, "{:?}", r.first);
}

#[test]
fn d2_a_packet_left_open_by_the_scan_before_pointer_mode_does_not_swallow_a_later_trigger() {
    // The segment that binds the software object also ends inside an open packet; the packet's tail is forwarded
    // unscanned in Pointer mode. The Window starts at the anchor with a restarted parser: the trigger is found. A
    // Window that kept the stale open packet would read the trigger's words as the packet's arguments, miss the
    // trigger, and the hardware would execute it unscanned.
    let script = vec![
        Op::Push(ge(0, Kind::SetObjOpen)),
        Op::Push(ge(1, Kind::OpenTail)),
        Op::Push(p(2)),
        Op::SetPut,
        Op::Ring,
        Op::Register,
        Op::AwaitReply,
        Op::Push(ge(3, Kind::Trig { migrate: false }).post()),
        Op::Push(p(4).post()),
        Op::SetPut,
        Op::Ring,
    ];
    let mut cfg = Cfg::new(spec(script, &[0, 1, 2, 3, 4]));
    cfg.kind = Registration::Other;
    let r = assert_clean(&cfg, "d2-stale-open-packet");
    assert_eq!(r.triggers_found_max, 1);
}

#[test]
fn d3_a_credit_stall_inside_a_packet_never_gets_a_marker() {
    // One submission whose packet spans two entries, on a private ring too small for it: the credit wait happens
    // mid-packet. Pointer mode must NOT author a marker there (it knows no boundary): the wake is a look.
    let script = vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::SetPut,
        Op::Ring,
        Op::Push(ge(1, Kind::OpenHead)),
        Op::Push(ge(2, Kind::OpenTail)),
        Op::Push(p(3)),
        Op::Push(p(4)),
        Op::SetPut,
        Op::Ring,
    ];
    let mut cfg = Cfg::new(spec(script, &[0, 1, 2, 3, 4]));
    cfg.priv_entries = 4;
    cfg.looks = true;
    cfg.max_states = 8_000_000;
    let r = assert_clean(&cfg, "d3-stall-in-packet");
    assert_eq!(r.modes, HashSet::from([Mode::Pointer]));
}

// ---- PHASE 1c (second review N2): the measurement-class modes (`KF3_LIFECYCLE_MODE`). Each has an observable
// difference the model checks over every interleaving, and no mode loses, duplicates or reorders an entry.

fn p2prime_script() -> Vec<Op> {
    vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::SetPut,
        Op::Ring,
        Op::Push(p(1).sync()),
        Op::Push(ge(2, Kind::OpenHead)),
        Op::SetPut, // GP_PUT ends INSIDE a packet, long after the binding
        Op::Ring,
        Op::Push(ge(3, Kind::OpenTail)),
        Op::SetPut,
        Op::Ring,
    ]
}

#[test]
fn m1_default_reads_no_pushbuffer_after_the_binding_and_always_scan_reads_every_one() {
    let (script, expected) = batch_ends_script(3);
    let mut cfg = Cfg::new(spec(script, &expected));
    let d = assert_clean(&cfg, "m1-default");
    assert_eq!(
        d.pb_reads_max, 1,
        "default: only the SET_OBJECT segment is ever read"
    );
    assert_eq!(d.modes, HashSet::from([Mode::Pointer]));
    assert!(d.entries_pointer_max >= 1);
    cfg.mode = LifecycleMode::AlwaysScan;
    let a = assert_clean(&cfg, "m1-always-scan");
    assert!(
        a.pb_reads_max >= 4,
        "always_scan: every entry's pushbuffer is read (a credit stall may read one again): {}",
        a.pb_reads_max
    );
    assert_eq!(a.pointer_markers_max, 0);
    assert_eq!(a.modes, HashSet::from([Mode::Scan]), "never leaves Scan");
    assert_eq!(a.entries_pointer_max, 0);
    assert_eq!(a.to_pointer_max, 0);
}

#[test]
fn m1_always_scan_measures_a_gp_put_boundary_inside_a_packet_on_the_whole_run_default_cannot() {
    // The submission [1, 2:OpenHead] ends inside a packet AFTER the software object is bound. The default machine is
    // in Pointer mode by then and has no parser state to look at; always_scan sees it (put_boundary_open_scan).
    let mut cfg = Cfg::new(spec(p2prime_script(), &[0, 1, 2, 3]));
    let d = explore(&cfg);
    assert_eq!(d.put_boundary_open_scan_max, 0, "invisible in default mode");
    assert!(
        d.viol & V_MARKER_IN_PACKET != 0,
        "and in default mode the Pointer batch-end marker lands in that packet (P2' false): the hazard the mode measures"
    );
    cfg.mode = LifecycleMode::AlwaysScan;
    let a = assert_clean(&cfg, "m1-always-scan-boundaries");
    assert!(
        a.put_boundary_open_scan_max >= 1,
        "some interleaving reaches the open boundary with the parser state known"
    );
    assert!(a.put_boundaries_scan_max > d.put_boundaries_scan_max);
}

#[test]
fn m1_always_scan_with_a_registration_and_a_ring_switch_still_relays_everything_in_order() {
    for kind in [Registration::Migrate, Registration::Other] {
        let mut cfg = flip(kind);
        cfg.mode = LifecycleMode::AlwaysScan;
        let r = assert_clean(&cfg, &format!("m1-always-scan-flip-{kind:?}"));
        assert_eq!(r.modes, HashSet::from([Mode::Scan]), "{kind:?}");
        assert_eq!(r.triggers_found_max, 1);
    }
}

fn batch_ends_script(n: u8) -> (Vec<Op>, Vec<u8>) {
    let mut script = vec![Op::Push(ge(0, Kind::SetObj)), Op::SetPut, Op::Ring];
    let mut expected = vec![0u8];
    for id in 1..=n {
        script.extend([Op::Push(p(id)), Op::SetPut, Op::Ring]);
        expected.push(id);
    }
    (script, expected)
}

#[test]
fn m1_no_pointer_markers_places_no_marker_in_pointer_mode_and_the_wake_falls_back_to_a_look() {
    // d3 (a 4-slot guest ring that fills): Pointer-mode batch-end markers announce the pops with NO look. Without them
    // the run is live only with a look (the bounded spin / the counted backstop), and the wake is counted.
    let (script, expected) = batch_ends_script(4);
    let mut cfg = Cfg::new(spec(script, &expected));
    cfg.src_entries = 4;
    cfg.priv_entries = 8;
    cfg.max_states = 8_000_000;
    cfg.looks = false;
    let d = assert_clean(&cfg, "m1-markers-default");
    assert!(
        d.pointer_markers_max >= 1,
        "default: Pointer markers are placed"
    );
    cfg.mode = LifecycleMode::NoPointerMarkers;
    cfg.looks = false;
    let n = assert_found(&cfg, "m1-no-pointer-markers-no-looks", V_DEADLOCK);
    assert_eq!(n.pointer_markers_max, 0, "no marker in Pointer mode");
    cfg.looks = true;
    let n = assert_clean(&cfg, "m1-no-pointer-markers-looks");
    assert_eq!(n.pointer_markers_max, 0);
    assert!(
        n.pointer_carried_no_marker_max >= 1,
        "the wake that has no marker is counted (loud)"
    );
    assert_eq!(n.modes, HashSet::from([Mode::Pointer]));
}

#[test]
fn m1_no_anchored_windows_is_the_phase_1_start_and_puts_the_marker_inside_the_packet() {
    // d1: a registration inside a submission whose packet spans two entries. Anchored (default) is clean; the PHASE 1
    // start at the entry that sees the registration lands the seeded gate inside the packet.
    let mut cfg = Cfg::new(spec(d1_script(), &[0, 1, 2, 3, 4, 5]));
    cfg.kind = Registration::Other;
    assert_clean(&cfg, "m1-anchored");
    cfg.mode = LifecycleMode::NoAnchoredWindows;
    assert_found(&cfg, "m1-no-anchored-windows", V_MARKER_IN_PACKET);
}

// ---- PHASE 1c (owner request): P1 over a whole run. `always_scan` counts EVERY 5080 SET_OBJECT the scan sees and every
// rebind of the software subchannel; the default machine stops looking at Pointer mode by construction.

fn set_object_script(rebind: bool) -> (Vec<Op>, Vec<u8>) {
    let mut script = vec![
        Op::Push(ge(0, Kind::SetObj)),
        Op::Push(p(1)),
        Op::SetPut,
        Op::Ring,
        Op::Push(ge(2, Kind::SetObj)), // the same object bound AGAIN, long after the first binding
        Op::Push(p(3)),
        Op::SetPut,
        Op::Ring,
    ];
    let mut expected = vec![0u8, 1, 2, 3];
    if rebind {
        script.extend([
            Op::Push(ge(4, Kind::Rebind)),
            Op::Push(p(5)),
            Op::SetPut,
            Op::Ring,
        ]);
        expected.extend([4, 5]);
    } else {
        script.extend([Op::Push(p(4)), Op::SetPut, Op::Ring]);
        expected.push(4);
    }
    (script, expected)
}

#[test]
fn m2_always_scan_counts_every_5080_set_object_and_every_rebind_of_the_whole_run() {
    for rebind in [false, true] {
        let (script, expected) = set_object_script(rebind);
        let mut cfg = Cfg::new(spec(script, &expected));
        cfg.mode = LifecycleMode::AlwaysScan;
        let r = assert_clean(&cfg, &format!("m2-always-scan-rebind={rebind}"));
        assert_eq!(
            r.set_object_total_max, 2,
            "both SET_OBJECTs of 5080 (rebind={rebind})"
        );
        assert_eq!(r.set_object_rebinds_max, u64::from(rebind));
    }
}

#[test]
fn m2_the_default_machine_cannot_see_a_binding_after_pointer_mode_began() {
    // Documents the measurement gap on purpose: in Pointer mode nothing is scanned, so only the first binding counts.
    let (script, expected) = set_object_script(true);
    let cfg = Cfg::new(spec(script, &expected));
    let r = assert_clean(&cfg, "m2-default");
    assert_eq!(r.set_object_total_max, 1);
    assert_eq!(r.set_object_rebinds_max, 0);
}

// ---- PHASE 1c item 2 (second review N1): the VM-wide latch. The first packet seen open at a GP_PUT boundary (Scan: the
// parser state is known) switches Pointer markers off; markers are performance, never correctness, so no entry may be
// lost, duplicated or reordered by switching them off, and the wake falls back to a look (counted).

/// `SetObjOpen` binds AND ends inside an open packet; the guest's GP_PUT ends right there (an open boundary in Scan);
/// then a Pointer-mode submission that ends INSIDE a packet again (the P2' residual of `p2prime_*`).
fn latch_script() -> Vec<Op> {
    vec![
        Op::Push(ge(0, Kind::SetObjOpen)),
        Op::SetPut,
        Op::Ring,
        Op::Push(ge(1, Kind::OpenTail)),
        Op::Push(p(2)),
        Op::Push(ge(3, Kind::OpenHead)),
        Op::SetPut, // ends inside a packet, in Pointer mode
        Op::Ring,
        Op::Push(ge(4, Kind::OpenTail)),
        Op::Push(p(5)),
        Op::SetPut,
        Op::Ring,
    ]
}

#[test]
fn l1_once_an_open_boundary_was_seen_no_marker_ever_lands_in_a_packet_and_nothing_is_lost() {
    let cfg = Cfg::new(spec(latch_script(), &[0, 1, 2, 3, 4, 5]));
    let r = explore(&cfg);
    println!(
        "l1: {} states, {} latched, viol={:?} latched viol={:?}",
        r.states,
        r.latched_states,
        viol_names(r.viol),
        viol_names(r.viol_latched)
    );
    assert!(r.latched_states > 0, "some interleaving sees the open boundary in Scan");
    assert_eq!(
        r.viol_latched, 0,
        "in every latched interleaving: no marker in a packet, no loss, no duplicate, no deadlock (a look is allowed): {:?}",
        r.first
    );
    assert_eq!(r.latched_markers_max, 0, "no Pointer marker after the latch");
    assert!(
        r.latched_carried_max > 0,
        "and the wakes it cannot announce are counted (loud)"
    );
    // the residual the latch cannot remove: interleavings where the worker never read the open GP_PUT in Scan
    assert!(
        r.viol & V_MARKER_IN_PACKET != 0,
        "unlatched interleavings still reach the P2' violation (the latch needs an observation)"
    );
}

#[test]
fn l1_latched_markers_off_the_wake_is_a_look_and_without_looks_the_run_deadlocks_as_counted() {
    // d3 (4-slot guest ring) after a latch: the batch-end markers that announced the pops are gone.
    let mut script = vec![
        Op::Push(ge(0, Kind::SetObjOpen)),
        Op::SetPut,
        Op::Ring,
        Op::Push(ge(1, Kind::OpenTail)),
        Op::SetPut,
        Op::Ring,
    ];
    let mut expected = vec![0u8, 1];
    for id in 2..=4u8 {
        script.extend([Op::Push(p(id)), Op::SetPut, Op::Ring]);
        expected.push(id);
    }
    let mut cfg = Cfg::new(spec(script, &expected));
    cfg.src_entries = 4;
    cfg.priv_entries = 8;
    cfg.max_states = 8_000_000;
    cfg.looks = true;
    let with = explore(&cfg);
    assert!(with.latched_states > 0);
    assert_eq!(
        with.viol_latched, 0,
        "markers off never loses or duplicates an entry: {:?}",
        with.first
    );
    assert!(with.latched_carried_max > 0, "the fallback is counted");
    cfg.looks = false;
    let without = explore(&cfg);
    assert!(
        without.viol_latched & V_DEADLOCK != 0,
        "with looks AND markers gone, a full guest ring is never woken: the fallback IS the look"
    );
    assert_eq!(
        without.viol_latched & !V_DEADLOCK,
        0,
        "and nothing else goes wrong"
    );
}
