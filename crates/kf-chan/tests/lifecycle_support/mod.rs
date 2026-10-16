//! Shared test support for the USER-ring lifecycle (`docs/design/V3_USER_RING_LIFECYCLE_20261016.md`):
//! ONE transliteration of the relay's per-entry turn (`crates/kf-qemu/src/userpump.rs`
//! `Pump::turn_body` / `segment_turn` / `end_batch` / `credit_need`, line references in the comments)
//! against an abstract [`Env`], driven three ways: the exhaustive interleaving model
//! (`user_lifecycle_model.rs`), the sequential property / differential tests
//! (`user_lifecycle_props.rs`) and the real-thread hostile-writer stress (`user_lifecycle_stress.rs`).
//!
//! The turn uses the REAL `Lifecycle`, `SyncGate`, `Marks`, `Grace`, `Scan`, `Fetch`, `inspect_entry` and
//! `Wait`; only the I/O (`Pump::snapshot`, `publish`, the USERD accesses) and the lock handling are
//! abstracted. A turn is split into micro-steps so that a driver may run any other actor between any
//! two of them (the guest changes ONE dword at any instant, the registry epoch moves, the PBDMA pops).
#![allow(dead_code, clippy::too_many_lines, clippy::type_complexity, missing_docs)]

use kf_abi::submit::{gp_entry, method_header_inc};
use kf_chan::user_gate::{Gate, Grace, Marks, SyncGate};
use kf_chan::user_lifecycle::{Lifecycle, Mode, Obs, Plan, ScanPermit};
use kf_chan::user_loop::{Need, Wait, wait_of};
use kf_chan::user_scan::{Fetch, Scan};
use kf_chan::user_stream::{Source, inspect_entry};

/// The software object's class value the scan binds (`Software::matches`).
pub const SW: u32 = 0x5080;
/// The registered handle a trigger carries.
pub const HANDLE: u32 = 0x4000_002c;
/// What an unwritten pushbuffer word holds: a header that does not decode (`Refusal::Header`).
pub const POISON: u32 = 0xC000_1234;
/// GP_ENTRY1 SYNC_WAIT (bit 31 of the high dword).
pub const SYNC: u64 = 1 << 63;

pub fn sw(v: u32) -> bool {
    v == SW
}

fn h(sub: u32, m: u32, n: u32) -> u32 {
    method_header_inc(sub, m, n).expect("header")
}

// ------------------------------------------------------------------------------------------------
// guest entries and their pushbuffers

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// `[INC sub0 0x100 x1, id]`: one complete packet.
    Plain,
    /// A control entry (`LENGTH == 0`): no pushbuffer.
    Nop,
    /// `[SET_OBJECT sub6 = 5080]`.
    SetObj,
    /// Header only of the SET_OBJECT packet; the datum is the next entry (`SetObjTail`).
    SetObjHead,
    /// The datum of the split SET_OBJECT packet.
    SetObjTail,
    /// `[INC sub1 0x1b0 x3, a]`: ends inside an open packet (1 of 3 arguments).
    OpenHead,
    /// `[b, c]`: the remaining two arguments of an `OpenHead`.
    OpenTail,
    /// `[INC sub6 0x200 x1, HANDLE]`: the software-method trigger. `migrate` = the registered bundle is a
    /// `FIFO_UPDATE_CHANNEL_INFO` (the source switches), else a TLB-invalidate-like bundle.
    Trig {
        migrate: bool,
    },
    /// `[SET_OBJECT sub6 = other class]`: rebinds the software subchannel.
    Rebind,
    /// `[SET_OBJECT sub7 = 5080]`: the software object on another subchannel.
    SetObj7,
    /// A trigger-shaped packet on the WRONG subchannel (`sub5`), which the scan does not call a trigger.
    TrigWrongSub,
    /// A packet whose DATA word equals the trigger pattern inside another method's arguments:
    /// `[INC sub0 0x100 x3, h(6,0x200,1), HANDLE, 7]`.
    FalseTrig,
    /// A trigger split over two entries: `[h(6,0x200,1)]` then `[HANDLE]`.
    TrigHead,
    TrigTail,
    /// `[INC sub7 0x200 x1, HANDLE]`: a trigger on the OTHER subchannel.
    Trig7,
    /// `[INC sub0 0x100 x3]` header only: the data that follows looks like the trigger.
    FalseHead,
    /// `[h(6,0x200,1), HANDLE, 7]`: the two arguments (and one header-looking word) of a `FalseHead`.
    FalseTail,
    /// The remaining arguments of an `OpenHead`, but words that do not decode as a header.
    OpenTailBad,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ge {
    pub id: u8,
    pub kind: Kind,
    pub sync: bool,
    /// The segment ends with a semaphore acquire on this semaphore (the PBDMA blocks until released).
    pub acq: Option<u8>,
    /// The pushbuffer is written by an explicit `Fill`, not at `Push`.
    pub late: bool,
    /// Published after the guest received a registration's reply (checked by I4).
    pub post_reply: bool,
}

pub fn ge(id: u8, kind: Kind) -> Ge {
    Ge {
        id,
        kind,
        sync: false,
        acq: None,
        late: false,
        post_reply: false,
    }
}
impl Ge {
    pub fn sync(self) -> Self {
        Self { sync: true, ..self }
    }
    pub fn acq(self, s: u8) -> Self {
        Self {
            acq: Some(s),
            ..self
        }
    }
    pub fn late(self) -> Self {
        Self { late: true, ..self }
    }
    pub fn post(self) -> Self {
        Self {
            post_reply: true,
            ..self
        }
    }
    /// The pushbuffer words as the guest finally writes them.
    pub fn words(&self) -> Vec<u32> {
        match self.kind {
            Kind::Plain => vec![h(0, 0x100, 1), u32::from(self.id)],
            Kind::Nop => vec![],
            Kind::SetObj => vec![h(6, 0, 1), SW],
            Kind::SetObjHead => vec![h(6, 0, 1)],
            Kind::SetObjTail => vec![SW],
            Kind::OpenHead => vec![h(1, 0x1b0, 3), 0x11],
            Kind::OpenTail => vec![0x22, 0x33],
            Kind::Trig { .. } => vec![h(6, 0x200, 1), HANDLE],
            Kind::Rebind => vec![h(6, 0, 1), 0xc7c0],
            Kind::SetObj7 => vec![h(7, 0, 1), SW],
            Kind::TrigWrongSub => vec![h(5, 0x200, 1), HANDLE],
            Kind::FalseTrig => vec![h(0, 0x100, 3), h(6, 0x200, 1), HANDLE, 7],
            Kind::TrigHead => vec![h(6, 0x200, 1)],
            Kind::TrigTail => vec![HANDLE],
            Kind::Trig7 => vec![h(7, 0x200, 1), HANDLE],
            Kind::FalseHead => vec![h(0, 0x100, 3)],
            Kind::FalseTail => vec![h(6, 0x200, 1), HANDLE, 7],
            Kind::OpenTailBad => vec![POISON, 0x33],
        }
    }
    /// The 8-byte GP entry for this entry (a real C56F entry; the pushbuffer VA names the id).
    pub fn raw(&self) -> u64 {
        let s = if self.sync { SYNC } else { 0 };
        if self.kind == Kind::Nop {
            return s | (u64::from(self.id) << 2);
        }
        gp_entry(seg_va(self.id), self.words().len() as u64 * 4).expect("encodable") | s
    }
    pub fn is_trigger(&self) -> bool {
        matches!(self.kind, Kind::Trig { .. } | Kind::Trig7 | Kind::TrigTail)
    }
}

pub fn seg_va(id: u8) -> u64 {
    0x1000 * (u64::from(id) + 1)
}

/// Which guest entry id an 8-byte GP entry names (`None` for a value that names none).
pub fn entry_id(raw: u64) -> Option<u8> {
    match inspect_entry(raw, 1 << 20).ok()? {
        Source::Nop => u8::try_from((raw & 0xffff_ffff) >> 2).ok(),
        Source::Segment(d) => u8::try_from(d.gpu_va / 0x1000 - 1).ok(),
    }
}

// ------------------------------------------------------------------------------------------------
// the environment

/// What the relay publishes on the private ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pub {
    /// The guest's entry as the relay stores it. `tested` = the value the relay validated, `stored` =
    /// the value it wrote (I7: they must be the same). `scanned` = its pushbuffer was read and scanned.
    Guest {
        tested: u64,
        stored: u64,
        scanned: bool,
    },
    /// A SYNC_WAIT control entry.
    Barrier,
    /// A tagged marker segment (`SET_REFERENCE` + `NON_STALL_INTERRUPT`); the value is the marker count.
    Marker(u64),
    /// The staged copy of a trigger segment (the trigger itself neutralised).
    Staged { tested: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ev {
    /// `GP_PUT` outside the ring.
    PutBound,
    /// `inspect_entry` refused the loaded value.
    EntryRefused,
    /// The scan refused a segment for good (the channel is dead).
    ScanRefused,
    /// A SYNC_WAIT entry behind an open packet was read at once.
    CarriedUngated,
    /// The relay re-read a refused segment (`Grace`).
    Retry,
    /// A trigger was found by the scan.
    TriggerFound,
    /// The source was switched.
    Switched,
}

pub trait Env {
    fn src_entries(&self) -> u32;
    fn gp_put(&mut self) -> u32;
    /// One dword of the current source ring's slot (`hi` = the high dword).
    fn gp_dword(&mut self, slot: u32, hi: bool) -> u32;
    fn registry_epoch(&mut self) -> u64;
    fn registry_live(&mut self) -> u32;
    fn hw_get(&mut self) -> u32;
    fn hw_reference(&mut self) -> u32;
    /// Read the pushbuffer of the guest entry `raw` (names an id): the words as they are NOW.
    fn read_pb(&mut self, raw: u64) -> Vec<u32>;
    fn publish(&mut self, p: Pub);
    fn write_guest_get(&mut self, v: u32);
    /// Whether the registered bundle the trigger entry `raw` names is a ring growth (the source switches).
    fn trigger_migrates(&mut self, raw: u64) -> bool;
    /// The trigger's action ran: the registration is consumed (implicit delete); `migrate` = switch source.
    fn executed(&mut self, migrate: bool);
    fn event(&mut self, e: Ev);
}

// ------------------------------------------------------------------------------------------------
// the relay

/// Deliberate defects of the GLUE (the turn), for the mutation checks. The machine's own mutants are
/// `kf_chan::user_lifecycle::Mutant` (feature `lifecycle-mutants`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Glue {
    /// Load the registry epoch BEFORE the entry (the wrong order).
    pub epoch_before_entry: bool,
    /// Store a second load of the entry instead of the tested value.
    pub double_fetch: bool,
    /// Do not reset the SyncGate at a Fresh restart.
    pub no_gate_reset: bool,
    /// Define the flip boundary by a re-read of `GP_PUT` instead of the cursor.
    pub flip_at_put: bool,
    /// Do not apply the seeded gate (ignore `seed_gate`).
    pub ignore_seed: bool,
    /// The worker never LOOKS at its waits on an event (only a timer would): progress depends on time.
    pub ignore_events: bool,
    /// Drop the first entry of a Fresh window (advance the cursor without publishing it).
    pub drop_flip_entry: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Phase {
    Idle,
    Obs,
    Wb,
    SegTurn,
    Put,
    Lo,
    Hi,
    Epoch,
    Plan,
    PubPointer,
    Gate,
    PbRead,
    ScanStep,
    PubScan,
    Stage,
    Action,
}

#[derive(Clone, Debug)]
struct Staged {
    witness: u64,
    migrate: bool,
}

/// The cursor after the source switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cursor {
    Zero,
    Keep,
}

#[derive(Clone, Debug)]
pub struct Relay {
    pub life: Lifecycle,
    pub scan: Scan,
    pub next_scan: Scan,
    pub sync: SyncGate,
    pub marks: Marks,
    pub fetch: Fetch,
    pub grace: Grace,
    pub cursor: u32,
    pub gener: u64,
    pub abs: u64,
    pub phase: Phase,
    pub dead: bool,
    pub again: bool,
    pub waiting: Option<Wait>,
    pub unnotified: bool,
    pub marker_event: bool,
    pub gate_target: u64,
    pub wb: Option<(u64, u32)>,
    pub glue: Glue,
    pub cursor_after_switch: Cursor,
    // the entry in flight
    lo: u32,
    hi: u32,
    raw: u64,
    epoch: u64,
    plan: Option<Plan>,
    seed: bool,
    words: Vec<u32>,
    trigger_at: bool,
    scanned_this: bool,
    permit: Option<ScanPermit>,
    staged: Option<Staged>,
    until_abs: u64,
    pub refused_entry: u64,
    pub puts: u64,
}

impl Relay {
    pub fn new(priv_entries: u32, life: Lifecycle, scan: Scan, glue: Glue) -> Self {
        Self {
            life,
            scan,
            next_scan: Scan::default(),
            sync: SyncGate::default(),
            marks: Marks::default(),
            fetch: Fetch::new(priv_entries).expect("private ring"),
            grace: Grace::default(),
            cursor: 0,
            gener: 1,
            abs: 0,
            phase: Phase::Idle,
            dead: false,
            again: false,
            waiting: None,
            unnotified: false,
            marker_event: false,
            gate_target: 0,
            wb: None,
            glue,
            cursor_after_switch: Cursor::Zero,
            lo: 0,
            hi: 0,
            raw: 0,
            epoch: 0,
            plan: None,
            seed: false,
            words: Vec::new(),
            trigger_at: false,
            scanned_this: false,
            permit: None,
            staged: None,
            until_abs: 0,
            refused_entry: 0,
            puts: 0,
        }
    }

    /// The model-checking key: everything that decides behaviour, none of the counters.
    pub fn key(&self) -> String {
        format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{}|{}|{}|{:?}|{:?}|{:?}|{}|{}|{:?}|{}|{}|{}|{}|{:x}|{}|{:?}|{}|{:?}|{}|{:?}",
            self.life.state_key(),
            self.scan,
            self.sync,
            self.marks,
            self.fetch,
            self.grace,
            self.cursor,
            self.gener,
            self.abs,
            self.phase,
            self.dead,
            self.again,
            self.unnotified as u8,
            self.marker_event as u8,
            self.waiting,
            self.gate_target,
            self.wb.map_or(0, |w| w.1 + 1),
            self.lo,
            self.hi,
            self.raw,
            self.epoch,
            self.plan.map(|p| matches!(p, Plan::Forward)),
            self.seed as u8,
            self.staged.as_ref().map(|s| (s.witness, s.migrate)),
            self.until_abs,
            self.next_scan,
        )
    }

    pub fn idle(&self) -> bool {
        self.phase == Phase::Idle
    }

    /// Whether the relay has any reason to run a turn without a new doorbell.
    pub fn owes_turn(&self) -> bool {
        !self.dead && self.again
    }

    /// Whether the outstanding wait is over, from the visible hardware words (the worker's LOOK).
    pub fn wait_satisfied(&self, env: &mut impl Env) -> bool {
        let Some(w) = self.waiting else {
            return false;
        };
        let reference = env.hw_reference();
        let get = env.hw_get();
        match self.fetch.peek(get) {
            Ok(popped) => w.satisfied(self.marks.peek(reference), popped),
            Err(_) => true,
        }
    }

    /// Start a turn (the claim). `flag` is the doorbell flag: cleared by the caller BEFORE the turn reads
    /// `GP_PUT` (the construction rule).
    pub fn claim(&mut self) {
        debug_assert_eq!(self.phase, Phase::Idle);
        self.again = false;
        self.waiting = None;
        self.phase = Phase::Obs;
    }

    fn marker_possible(&self) -> bool {
        self.life.marker_possible(self.scan.carried().is_some())
    }

    fn publish(&mut self, env: &mut impl Env, p: Pub) {
        env.publish(p);
        self.puts += 1;
    }

    fn publish_marker(&mut self, env: &mut impl Env) {
        let slot = self.fetch.publish(None).expect("credit");
        let _ = slot;
        let (n, _) = self.marks.issue().expect("markers");
        self.publish(env, Pub::Marker(n));
        self.unnotified = false;
        self.marker_event = true;
    }

    /// `Pump::credit_need` (userpump.rs `credit_need`).
    fn credit_need(&mut self, env: &mut impl Env) {
        let mut marker = self.marker_event;
        if self.unnotified {
            if self.marker_possible() && self.fetch.available() >= 1 {
                self.publish_marker(env);
                marker = true;
            } else {
                self.unnotified = false;
                self.marker_event = false;
                marker = false;
            }
        }
        self.waiting = wait_of(if marker {
            Need::Marker {
                target: self.marks.issued(),
            }
        } else {
            Need::Popped {
                target: self.fetch.fetched() + 1,
            }
        });
    }

    /// `Pump::end_batch`.
    fn end_batch(&mut self, env: &mut impl Env) {
        if self.unnotified {
            if !self.marker_possible() {
                self.unnotified = false;
                self.marker_event = false;
                self.waiting = wait_of(Need::Popped {
                    target: self.fetch.issued(),
                });
                return;
            }
            if self.fetch.available() >= 1 {
                self.publish_marker(env);
                self.waiting = wait_of(Need::Marker {
                    target: self.marks.issued(),
                });
                self.again = true;
                return;
            }
            self.waiting = wait_of(Need::Popped {
                target: self.fetch.fetched() + 1,
            });
            return;
        }
        if self.fetch.fetched() < self.fetch.issued() {
            self.waiting = wait_of(if self.marker_event {
                Need::Marker {
                    target: self.marks.issued(),
                }
            } else {
                Need::Popped {
                    target: self.fetch.issued(),
                }
            });
        }
    }

    /// One micro-step of the turn. Returns whether the relay did anything visible (for the caller's
    /// bookkeeping only).
    pub fn step(&mut self, env: &mut impl Env) {
        let entries = env.src_entries();
        match self.phase {
            Phase::Idle => {}
            Phase::Obs => {
                // userpump.rs turn_body: observe GP_GET / REFERENCE
                let get = env.hw_get();
                let reference = env.hw_reference();
                self.marks.observe(reference);
                for (g, gt) in self.fetch.observe(get).expect("observe") {
                    if g == self.gener {
                        self.wb = Some((g, gt));
                    }
                }
                self.phase = Phase::Wb;
            }
            Phase::Wb => {
                if let Some((g, gt)) = self.wb.take() {
                    if g == self.gener {
                        env.write_guest_get(gt);
                    }
                }
                self.phase = if self.staged.is_some() {
                    Phase::SegTurn
                } else {
                    Phase::Put
                };
            }
            Phase::SegTurn => {
                let w = self.staged.as_ref().expect("staged").witness;
                if self.marks.seen() < w {
                    self.waiting = wait_of(Need::Marker { target: w });
                    self.phase = Phase::Idle;
                } else {
                    self.phase = Phase::Action;
                }
            }
            Phase::Put => {
                let put = env.gp_put();
                if put >= entries {
                    env.event(Ev::PutBound);
                    self.dead = true;
                    self.phase = Phase::Idle;
                    return;
                }
                if put == self.cursor {
                    self.end_batch(env);
                    self.phase = Phase::Idle;
                    return;
                }
                self.phase = if self.glue.epoch_before_entry {
                    Phase::Epoch
                } else {
                    Phase::Lo
                };
            }
            Phase::Lo => {
                self.lo = env.gp_dword(self.cursor, false);
                self.phase = Phase::Hi;
            }
            Phase::Hi => {
                self.hi = env.gp_dword(self.cursor, true);
                self.raw = (u64::from(self.hi) << 32) | u64::from(self.lo);
                self.phase = if self.glue.epoch_before_entry {
                    Phase::Plan
                } else {
                    Phase::Epoch
                };
            }
            Phase::Epoch => {
                self.epoch = env.registry_epoch();
                self.phase = if self.glue.epoch_before_entry {
                    Phase::Lo
                } else {
                    Phase::Plan
                };
            }
            Phase::Plan => self.plan_step(env, entries),
            Phase::PubPointer => {
                if self.fetch.available() < 2 {
                    self.credit_need(env);
                    self.phase = Phase::Idle;
                    return;
                }
                self.publish_guest(env, entries, false);
                self.phase = Phase::Idle;
            }
            Phase::Gate => self.gate_step(env),
            Phase::PbRead => {
                let permit = self.permit.take().expect("permit");
                let words = env.read_pb(self.raw);
                self.life
                    .note_pb_read(&permit, words.len() * 4, false);
                self.words = words;
                self.scanned_this = true;
                self.phase = Phase::ScanStep;
            }
            Phase::ScanStep => {
                let mut next = self.scan.clone();
                match next.segment(&self.words, sw) {
                    Err(e) => {
                        let _ = e;
                        let first = !self.grace.retrying();
                        if first {
                            self.life.scan_refused();
                        }
                        if self.grace.refused(self.gener, self.cursor) {
                            env.event(Ev::Retry);
                            self.again = true;
                            self.phase = Phase::Idle;
                        } else {
                            env.event(Ev::ScanRefused);
                            self.dead = true;
                            self.phase = Phase::Idle;
                        }
                    }
                    Ok(t) => {
                        self.grace.clear();
                        self.next_scan = next;
                        self.trigger_at = t.is_some();
                        if t.is_some() {
                            env.event(Ev::TriggerFound);
                            self.phase = Phase::Stage;
                        } else {
                            self.phase = Phase::PubScan;
                        }
                    }
                }
            }
            Phase::PubScan => {
                if self.fetch.available() < 2 {
                    self.credit_need(env);
                    self.phase = Phase::Idle;
                    return;
                }
                let nop = self.words.is_empty();
                self.publish_guest(env, entries, true);
                self.scan = self.next_scan.clone();
                if !nop {
                    self.life
                        .segment_scanned(self.scan.carried().is_some());
                }
                self.phase = Phase::Idle;
            }
            Phase::Stage => {
                // barrier (carrying the entry's source receipt) + marker; the trigger neutralised
                if self.fetch.available() < 2 {
                    self.credit_need(env);
                    self.phase = Phase::Idle;
                    return;
                }
                let guest_get = (self.cursor + 1) & (entries - 1);
                self.fetch
                    .publish(Some((self.gener, guest_get)))
                    .expect("credit");
                self.publish(env, Pub::Staged { tested: self.raw });
                self.fetch.publish(None).expect("credit");
                let (n, _) = self.marks.issue().expect("markers");
                self.publish(env, Pub::Marker(n));
                self.scan = self.next_scan.clone();
                self.life
                    .segment_scanned(self.scan.carried().is_some());
                self.life.forwarded(self.abs, self.raw);
                self.abs += 1;
                self.cursor = guest_get;
                let migrate = env.trigger_migrates(self.raw);
                self.staged = Some(Staged {
                    witness: self.marks.issued(),
                    migrate,
                });
                self.waiting = wait_of(Need::Marker { target: n });
                self.phase = Phase::Idle;
            }
            Phase::Action => {
                let s = self.staged.take().expect("staged");
                env.executed(s.migrate);
                if s.migrate {
                    self.gener += 1;
                    self.cursor = match self.cursor_after_switch {
                        Cursor::Zero => 0,
                        Cursor::Keep => self.cursor,
                    };
                    self.grace.clear();
                    self.wb = None;
                    env.event(Ev::Switched);
                }
                self.again = true;
                self.phase = Phase::Idle;
            }
        }
    }

    fn plan_step(&mut self, env: &mut impl Env, entries: u32) {
        // entry-level validation on THE loaded value (userpump.rs: inspect_entry)
        let desc = match inspect_entry(self.raw, kf_chan::user_storage::MAX_SOURCE_BYTES) {
            Ok(d) => d,
            Err(_) => {
                env.event(Ev::EntryRefused);
                self.refused_entry += 1;
                self.dead = true;
                self.phase = Phase::Idle;
                return;
            }
        };
        let obs = Obs {
            raw: self.raw,
            abs: self.abs,
            epoch: self.epoch,
            sw_mask: self.scan.software_mask(sw),
            have_software: true,
            quiescent: self.staged.is_none() && !self.sync.waiting(),
        };
        let mut plan = self.life.entry(&obs, || env.registry_live());
        if self.glue.drop_flip_entry && matches!(plan, Plan::Scan { restart: true, .. }) {
            // a deliberate defect: the first entry of a Fresh window is skipped, never published
            self.cursor = (self.cursor + 1) & (entries - 1);
            self.abs += 1;
            self.again = true;
            self.phase = Phase::Idle;
            return;
        }
        if self.glue.flip_at_put {
            // the wrong definition of "published after the flip": everything up to a re-read of GP_PUT
            if let Plan::Scan { restart: true, .. } = plan {
                let put = env.gp_put();
                self.until_abs = self.abs + u64::from(put.wrapping_sub(self.cursor) & (entries - 1));
            }
            if self.abs < self.until_abs && self.life.mode() == Mode::Window {
                plan = Plan::Forward;
            }
        }
        match plan {
            Plan::Forward => {
                self.plan = Some(plan);
                self.phase = Phase::PubPointer;
            }
            Plan::Scan {
                restart,
                seed_gate,
                permit,
            } => {
                if restart {
                    self.scan = self.scan.fresh_resume();
                    if !self.glue.no_gate_reset {
                        self.sync = SyncGate::default();
                    }
                    self.grace.clear();
                    self.life.restarted();
                }
                self.seed = seed_gate && !self.glue.ignore_seed;
                self.permit = Some(permit);
                self.plan = Some(plan);
                self.phase = match desc {
                    Source::Nop => Phase::Gate,
                    Source::Segment(_) => Phase::Gate,
                };
            }
        }
    }

    /// userpump.rs: the SYNC_WAIT gate, then the control-entry / segment split.
    fn gate_step(&mut self, env: &mut impl Env) {
        let sync_bit = self.raw & SYNC != 0;
        let gate_raw = if self.seed { self.raw | SYNC } else { self.raw };
        let verdict = if gate_raw & SYNC != 0 && self.scan.carried().is_some() {
            env.event(Ev::CarriedUngated);
            Gate::Proceed
        } else {
            self.sync
                .entry(gate_raw, self.gener, self.cursor, self.marks.seen())
        };
        let _ = sync_bit;
        match verdict {
            Gate::Wait => {
                self.waiting = wait_of(Need::Marker {
                    target: self.gate_target,
                });
                self.phase = Phase::Idle;
            }
            Gate::PublishBarrier => {
                if self.fetch.available() < 2 {
                    self.credit_need(env);
                    self.phase = Phase::Idle;
                    return;
                }
                self.fetch.publish(None).expect("credit");
                self.publish(env, Pub::Barrier);
                self.fetch.publish(None).expect("credit");
                let (n, _) = self.marks.issue().expect("markers");
                self.publish(env, Pub::Marker(n));
                self.sync.published(n);
                self.gate_target = n;
                self.unnotified = false;
                self.marker_event = true;
                self.waiting = wait_of(Need::Marker { target: n });
                self.again = true;
                self.phase = Phase::Idle;
            }
            Gate::Proceed => {
                if self.seed {
                    self.life.gate_proven();
                    self.seed = false;
                }
                // a control entry publishes as it is; a segment is read and scanned
                if inspect_entry(self.raw, kf_chan::user_storage::MAX_SOURCE_BYTES)
                    == Ok(Source::Nop)
                {
                    self.permit = None;
                    self.phase = Phase::PubScan;
                    self.words = Vec::new();
                    self.next_scan = self.scan.clone();
                } else {
                    self.phase = Phase::PbRead;
                }
            }
        }
    }

    /// Publish the guest's entry (control entry, Pointer entry or scanned segment).
    fn publish_guest(&mut self, env: &mut impl Env, entries: u32, scanned: bool) {
        let guest_get = (self.cursor + 1) & (entries - 1);
        let stored = if self.glue.double_fetch {
            // a second load: the guest may have changed one dword since the first
            let lo = env.gp_dword(self.cursor, false);
            let hi = env.gp_dword(self.cursor, true);
            (u64::from(hi) << 32) | u64::from(lo)
        } else {
            self.raw
        };
        self.fetch
            .publish(Some((self.gener, guest_get)))
            .expect("credit");
        self.publish(
            env,
            Pub::Guest {
                tested: self.raw,
                stored,
                scanned,
            },
        );
        self.life.forwarded(self.abs, self.raw);
        self.abs += 1;
        self.cursor = guest_get;
        self.unnotified = true;
        self.again = true;
    }
}
