//! Shared test support for the USER-ring lifecycle (`docs/design/V3_USER_RING_LIFECYCLE_20261016.md`).
//!
//! PHASE 1b (review T1): the per-entry turn is PRODUCTION code now (`kf_chan::user_turn::Relay`, the same code the
//! USER-ring pump in `kf-qemu` runs). This module wraps it with simulated accessors ([`Env`] -> `Io`) and carries
//! the test-side stand-in of the pump's segment machinery (publication of a scanned segment, the staged trigger
//! path, the software action and the ring switch: `PubScan` / `Stage` / `Action`), which is the only transliterated
//! part left. It is driven three ways: the exhaustive interleaving model (`user_lifecycle_model.rs`), the sequential
//! property / differential tests (`user_lifecycle_props.rs`) and the real-thread hostile-writer stress
//! (`user_lifecycle_stress.rs`). A turn is a machine of micro-steps so that a driver may run any other actor between
//! any two of them (the guest changes ONE dword at any instant, the registry epoch moves, the PBDMA pops).
#![allow(
    dead_code,
    clippy::too_many_lines,
    clippy::type_complexity,
    missing_docs,
    clippy::collapsible_if
)]

use kf_abi::submit::{gp_entry, method_header_inc};
use kf_chan::user_gate::Marks;
use kf_chan::user_lifecycle::Lifecycle;
use kf_chan::user_loop::{Need, Wait, wait_of};
use kf_chan::user_scan::{Fetch, Scan};
use kf_chan::user_stream::{Source, inspect_format};
use kf_chan::user_turn::{self as turn, Fail, GpSegment, Io, PubKind, Staged, Step, SwView, Turn};
use std::ops::{Deref, DerefMut};

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
    /// `[SET_OBJECT sub6 = 5080, INC sub1 0x1b0 x3, a]`: binds AND ends inside an open packet (1 of 3 arguments).
    SetObjOpen,
    /// `[STORE_SUBDEVICE_MASK 0x001]`: a header-only control for sub-device 1.
    MaskStore,
    /// `[USE_SUBDEVICE_MASK]`: refused by a scan that has nothing stored.
    MaskUse,
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
            Kind::SetObjOpen => vec![h(6, 0, 1), SW, h(1, 0x1b0, 3), 0x11],
            Kind::MaskStore => vec![0x20000 | (1 << 4)],
            Kind::MaskUse => vec![0x30000],
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
    match inspect_format(raw).ok()? {
        Source::Nop => u8::try_from((raw & 0xffff_ffff) >> 2).ok(),
        Source::Segment(d) => u8::try_from((d.gpu_va / 0x1000).checked_sub(1)?).ok(),
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
    /// I8: the relay read a ring entry (or the pushbuffer of an entry) it had no right to read: an index at or
    /// beyond the `GP_PUT` it loaded in this turn, a second `GP_PUT` load, or a pushbuffer of an entry it did not load.
    BeyondPut,
}

pub trait Env {
    /// Test-side production-adapter hook, default inert for all historical scenarios.
    fn publication_ready(&self) -> bool {
        true
    }

    /// VA apply pending; default keeps the historical scenarios unchanged.
    fn walk_busy(&self) -> bool {
        false
    }
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
    /// The software object the relay holds was freed (default: never).
    fn software_gone(&self) -> bool {
        false
    }
    /// The guest `GP_GET` write finds the USERD admission / pt lock busy (`Ok(false)`: look again). Default: never.
    fn write_guest_get_busy(&mut self) -> bool {
        false
    }
    /// ★ lazy observe (A): the relay uses `put` WITHOUT having read `GP_PUT` this turn. Is it a value the guest really
    /// wrote, that is not beyond the guest's current `GP_PUT` (forward from `cursor`)? Default: not checked.
    fn cached_put_ok(&mut self, _put: u32, _cursor: u32) -> bool {
        true
    }
}

/// Harness knobs (the production code has no test switches; code mutants are source patches applied by
/// `scripts/lifecycle_mutation_check.sh`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Glue {
    /// The worker never LOOKS at its waits on an event (only a timer would): progress depends on time.
    pub ignore_events: bool,
}
impl Glue {
    pub fn from_env() -> Self {
        Self::default()
    }
}

/// The cursor after the source switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cursor {
    Zero,
    Keep,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Sub {
    /// The relay core runs (`Relay::step`).
    Core,
    SegTurn,
    PubScan,
    Stage,
    Action,
}

#[derive(Clone, Debug)]
struct Staged2 {
    witness: u64,
    migrate: bool,
}

/// What the simulated `Io::stage` found (read back by the driver).
#[derive(Clone, Debug, Default)]
struct StageOut {
    next_scan: Scan,
    trigger: bool,
}

struct EnvIo<'a, E: Env> {
    env: &'a mut E,
    out: &'a mut StageOut,
    validated: &'a mut u64,
    /// I8 bookkeeping of the turn: the cursor, the `GP_PUT` loaded (once), the entry loaded.
    cursor: u32,
    turn: &'a mut TurnIo,
}

/// What the stub ring / store remembers of the current turn (invariant I8).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TurnIo {
    put: Option<u32>,
    lo: Option<u32>,
    loaded: Option<u64>,
}

impl<E: Env> Io for EnvIo<'_, E> {
    fn walk_busy(&mut self) -> bool {
        self.env.walk_busy()
    }
    fn hw_words(&mut self) -> Turn<(u32, u32)> {
        let get = self.env.hw_get();
        let reference = self.env.hw_reference();
        Ok((get, reference))
    }
    fn write_guest_get(&mut self, get: u32) -> Turn<bool> {
        if self.env.write_guest_get_busy() {
            return Ok(false);
        }
        self.env.write_guest_get(get);
        Ok(true)
    }
    fn src_entries(&mut self) -> u32 {
        self.env.src_entries()
    }
    fn max_scan_bytes(&mut self) -> u64 {
        kf_chan::user_storage::MAX_SOURCE_BYTES
    }
    fn gp_put(&mut self) -> Turn<Option<u32>> {
        if self.turn.put.is_some() {
            self.env.event(Ev::BeyondPut); // GP_PUT is loaded ONCE per turn
        }
        let put = self.env.gp_put();
        self.turn.put = Some(put);
        Ok(Some(put))
    }
    fn begin_entry(&mut self, put: u32) {
        // A turn that did not read `GP_PUT` (rule A) used a remembered one: it must still be a value at or below the
        // guest's current `GP_PUT`, and it is the `GP_PUT` the I8 bookkeeping holds the loads against.
        if self.turn.put.is_none() {
            if !self.env.cached_put_ok(put, self.cursor) {
                self.env.event(Ev::BeyondPut);
            }
            self.turn.put = Some(put);
        }
    }
    fn load_dword(&mut self, slot: u32, hi: bool) -> Turn<u32> {
        // I8: only the slot at the cursor, and only while the loaded GP_PUT is ahead of it
        let n = self.env.src_entries();
        let ahead = self
            .turn
            .put
            .map(|p| p.wrapping_sub(slot) & (n - 1))
            .unwrap_or(0);
        if slot != self.cursor || ahead == 0 {
            self.env.event(Ev::BeyondPut);
        }
        let v = self.env.gp_dword(slot, hi);
        if hi {
            self.turn.loaded = self.turn.lo.map(|lo| (u64::from(v) << 32) | u64::from(lo));
        } else {
            self.turn.lo = Some(v);
        }
        Ok(v)
    }
    fn registry_epoch(&mut self) -> u64 {
        self.env.registry_epoch()
    }
    fn registry_live(&mut self) -> u32 {
        self.env.registry_live()
    }
    fn software(&mut self, _refresh: bool) -> Turn<SwView> {
        Ok(SwView {
            have: true,
            gone: self.env.software_gone(),
        })
    }
    fn sw_matches(&self, value: u32) -> bool {
        sw(value)
    }
    fn read_pb(&mut self, entry: u64, _seg: &GpSegment) -> Turn<(Vec<u32>, bool)> {
        if self.turn.loaded != Some(entry) {
            self.env.event(Ev::BeyondPut); // a pushbuffer only for the entry loaded in this turn
        }
        Ok((self.env.read_pb(entry), false))
    }
    fn stage(&mut self, scan: &Scan, words: &[u32]) -> Turn<Staged> {
        let mut next = scan.clone();
        match next.segment(words, sw) {
            Err(e) => Err(Fail::Refused(format!("USER_RING_SCAN {e:?}"))),
            Ok(t) => {
                self.out.next_scan = next;
                self.out.trigger = t.is_some();
                Ok(Staged { rare: t.is_some() })
            }
        }
    }
    fn validated(&mut self, entry: u64) {
        *self.validated = entry;
    }
    fn publish_entry(
        &mut self,
        kind: PubKind,
        fetch: &mut Fetch,
        generation: u64,
        guest_get: u32,
        entry: u64,
    ) -> Turn<()> {
        if !self.env.publication_ready() {
            return Err(Fail::Retry);
        }
        fetch
            .publish(Some((generation, guest_get)))
            .map_err(str::to_owned)?;
        self.env.publish(Pub::Guest {
            tested: *self.validated,
            stored: entry,
            scanned: kind == PubKind::Control,
        });
        Ok(())
    }
    fn publish_gate(&mut self, fetch: &mut Fetch, marks: &mut Marks) -> Turn<u64> {
        if !self.env.publication_ready() {
            return Err(Fail::Retry);
        }
        fetch.publish(None).map_err(str::to_owned)?;
        self.env.publish(Pub::Barrier);
        fetch.publish(None).map_err(str::to_owned)?;
        let (n, _) = marks.issue().map_err(str::to_owned)?;
        self.env.publish(Pub::Marker(n));
        Ok(n)
    }
    fn publish_marker(&mut self, fetch: &mut Fetch, marks: &mut Marks) -> Turn<()> {
        if !self.env.publication_ready() {
            return Err(Fail::Retry);
        }
        fetch.publish(None).map_err(str::to_owned)?;
        let (n, _) = marks.issue().map_err(str::to_owned)?;
        self.env.publish(Pub::Marker(n));
        Ok(())
    }
    fn event(&mut self, e: turn::Ev) {
        match e {
            turn::Ev::PutBound => self.env.event(Ev::PutBound),
            turn::Ev::EntryRefused => self.env.event(Ev::EntryRefused),
            turn::Ev::ScanRefused => self.env.event(Ev::ScanRefused),
            turn::Ev::ScanBudget => {}
            turn::Ev::CarriedUngated { .. } => self.env.event(Ev::CarriedUngated),
            turn::Ev::Retry { .. } => self.env.event(Ev::Retry),
            turn::Ev::GatePublished | turn::Ev::GateProceeded | turn::Ev::CreditWait => {}
        }
    }
}

/// The relay under test: the PRODUCTION per-entry turn plus the test-side stand-in of the pump's segment machinery.
#[derive(Clone, Debug)]
pub struct Relay {
    pub core: turn::Relay,
    sub: Sub,
    staged: Option<Staged2>,
    out: StageOut,
    validated: u64,
    turn_io: TurnIo,
    pub cursor_after_switch: Cursor,
}

impl Deref for Relay {
    type Target = turn::Relay;
    fn deref(&self) -> &Self::Target {
        &self.core
    }
}
impl DerefMut for Relay {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.core
    }
}

impl Relay {
    /// Opt the relay under test in to lazy observe (`Relay::new` leaves both rules OFF).
    #[must_use]
    pub fn with_lazy(mut self, cfg: kf_chan::user_lazy::LazyConfig) -> Self {
        self.core = self.core.with_lazy(cfg);
        self
    }

    pub fn new(priv_entries: u32, life: Lifecycle, scan: Scan, _glue: Glue) -> Self {
        Self {
            core: turn::Relay::new(priv_entries, life, scan).expect("private ring"),
            sub: Sub::Core,
            staged: None,
            out: StageOut::default(),
            validated: 0,
            turn_io: TurnIo::default(),
            cursor_after_switch: Cursor::Zero,
        }
    }

    /// The model-checking key: everything that decides behaviour, none of the counters.
    pub fn key(&self) -> String {
        format!(
            "{}|{:?}|{:?}|{:?}|{}",
            self.core.state_key(),
            self.sub,
            self.staged.as_ref().map(|s| (s.witness, s.migrate)),
            self.out.next_scan,
            self.out.trigger
        ) + &format!("|{:?}", self.turn_io)
    }

    pub fn idle(&self) -> bool {
        self.core.phase == turn::Phase::Idle && self.sub == Sub::Core
    }

    /// Whether the outstanding wait, if any (from `need`).
    pub fn waiting(&self) -> Option<Wait> {
        wait_of(self.core.need)
    }
    pub fn clear_waiting(&mut self) {
        self.core.need = Need::Idle;
    }

    /// Whether the relay has any reason to run a turn without a new doorbell.
    pub fn owes_turn(&self) -> bool {
        !self.core.dead && self.core.again
    }

    /// Whether the outstanding wait is over, from the visible hardware words (the worker's LOOK).
    pub fn wait_satisfied(&self, env: &mut impl Env) -> bool {
        let Some(w) = self.waiting() else {
            return false;
        };
        let reference = env.hw_reference();
        let get = env.hw_get();
        match self.core.fetch.peek(get) {
            Ok(popped) => w.satisfied(self.core.marks.peek(reference), popped),
            Err(_) => true,
        }
    }

    /// Start a turn (the claim). The doorbell flag is cleared by the caller BEFORE the turn reads GP_PUT.
    pub fn claim(&mut self) {
        debug_assert!(self.idle());
        self.core.segment_pending = self.staged.is_some();
        self.turn_io = TurnIo::default();
        self.core.claim();
    }

    fn io<'a, E: Env>(
        env: &'a mut E,
        out: &'a mut StageOut,
        validated: &'a mut u64,
        cursor: u32,
        turn: &'a mut TurnIo,
    ) -> EnvIo<'a, E> {
        EnvIo {
            env,
            out,
            validated,
            cursor,
            turn,
        }
    }

    /// One micro-step of the turn.
    pub fn step(&mut self, env: &mut impl Env) {
        self.step_inner(env);
        // the pump does this at the end of every turn (the tally commits with the scan)
        self.core.sync_tally();
    }

    fn step_inner(&mut self, env: &mut impl Env) {
        if matches!(self.sub, Sub::PubScan | Sub::Stage | Sub::Action) && !env.publication_ready() {
            return;
        }
        match self.sub {
            Sub::Core => {
                let r = {
                    let mut io = Self::io(
                        env,
                        &mut self.out,
                        &mut self.validated,
                        self.core.cursor,
                        &mut self.turn_io,
                    );
                    self.core.step(&mut io)
                };
                match r {
                    Ok(Step::More | Step::Shadow) | Err(_) => {}
                    Ok(Step::Idle { .. }) => {}
                    Ok(Step::Segment) => self.sub = Sub::SegTurn,
                    Ok(Step::Scanned { rare }) => {
                        if rare {
                            env.event(Ev::TriggerFound);
                            self.sub = Sub::Stage;
                        } else {
                            self.sub = Sub::PubScan;
                        }
                    }
                }
            }
            Sub::SegTurn => {
                let w = self.staged.as_ref().expect("staged").witness;
                if self.core.marks.seen() < w {
                    self.core.need = Need::Marker { target: w };
                    self.sub = Sub::Core;
                } else {
                    self.sub = Sub::Action;
                }
            }
            Sub::PubScan => {
                if self.core.fetch.available() < 2 {
                    let mut io = Self::io(
                        env,
                        &mut self.out,
                        &mut self.validated,
                        self.core.cursor,
                        &mut self.turn_io,
                    );
                    let _ = self.core.credit_need(&mut io);
                    self.sub = Sub::Core;
                    return;
                }
                // the pump's segment_turn, non-rare path: publish, then commit the scan
                let get = self.core.next_get;
                self.core
                    .fetch
                    .publish(Some((self.core.generation, get)))
                    .expect("credit");
                env.publish(Pub::Guest {
                    tested: self.core.raw,
                    stored: self.core.raw,
                    scanned: true,
                });
                self.core.puts += 1;
                self.core.scan = self.out.next_scan.clone();
                let open = self.core.scan.carried().is_some();
                self.core.life.segment_scanned(open);
                self.core.cursor = get;
                self.core.note_relayed(self.core.raw);
                self.core.unnotified = true;
                self.core.again = true;
                self.sub = Sub::Core;
            }
            Sub::Stage => {
                // barrier (carrying the entry's source receipt) + marker; the trigger neutralised
                if self.core.fetch.available() < 2 {
                    let mut io = Self::io(
                        env,
                        &mut self.out,
                        &mut self.validated,
                        self.core.cursor,
                        &mut self.turn_io,
                    );
                    let _ = self.core.credit_need(&mut io);
                    self.sub = Sub::Core;
                    return;
                }
                let get = self.core.next_get;
                self.core
                    .fetch
                    .publish(Some((self.core.generation, get)))
                    .expect("credit");
                env.publish(Pub::Staged {
                    tested: self.core.raw,
                });
                self.core.fetch.publish(None).expect("credit");
                let (n, _) = self.core.marks.issue().expect("markers");
                env.publish(Pub::Marker(n));
                self.core.puts += 2;
                self.core.scan = self.out.next_scan.clone();
                let open = self.core.scan.carried().is_some();
                self.core.life.segment_scanned(open);
                self.core.note_relayed(self.core.raw);
                self.core.cursor = get;
                let migrate = env.trigger_migrates(self.core.raw);
                self.staged = Some(Staged2 {
                    witness: self.core.marks.issued(),
                    migrate,
                });
                self.core.need = Need::Marker { target: n };
                self.sub = Sub::Core;
            }
            Sub::Action => {
                let s = self.staged.take().expect("staged");
                env.executed(s.migrate);
                if s.migrate {
                    // the production switch (the pump's `migrate` calls the same method)
                    let keep = self.core.cursor;
                    self.core.switch_source(self.core.generation + 1);
                    if self.cursor_after_switch == Cursor::Keep {
                        self.core.cursor = keep;
                    }
                    env.event(Ev::Switched);
                }
                self.core.again = true;
                self.sub = Sub::Core;
            }
        }
    }
}
