//! ★ 2026-10-16 (PHASE 1b) — **the per-entry turn of the Windows USER-ring relay**, as PRODUCTION code behind an I/O
//! trait (`docs/design/V3_USER_RING_LIFECYCLE_20261016.md`, review finding T1).
//!
//! `crates/kf-qemu/src/userpump.rs` runs this code with the real accessors ([`Io`]); the interleaving model, the
//! property tests and the thread stress run THE SAME code with simulated ones (`tests/lifecycle_support`). Before
//! this module the tests drove a line-for-line transliteration, and an edit of the real pump (for example replacing
//! the seeded gate) was invisible to every suite.
//!
//! What is here: observe, guest `GP_GET` write-back, `GP_PUT` read, the entry load, the lifecycle plan, the
//! SYNC_WAIT gate, Pointer / control publication, the pushbuffer read, the scan and its bounded retry, and the
//! marker / credit rules (`end_batch`, `credit_need`). What is not: the publication of a SCANNED segment and the
//! staged trigger path (`segment_turn`, the software action, the ring switch). The pump keeps those; it takes them
//! over at [`Step::Scanned`] / [`Step::Segment`] and calls back into [`Relay::credit_need`] and
//! [`Relay::note_relayed`].
//!
//! The turn is a machine of micro-steps ([`Phase`]) so that a driver can run any other actor between any two loads.
//! Production runs the steps back to back; an error ends the turn (the in-flight entry is dropped and re-loaded by
//! the next turn, exactly as the straight-line code did).

use crate::user_gate::{Gate, Grace, Marks, SyncGate};
use crate::user_lifecycle::{GP_SYNC_WAIT, Lifecycle, Mode, Obs, Plan, ScanPermit};
use crate::user_loop::Need;
use crate::user_scan::{Fetch, Scan};
use crate::user_stream::{Source, inspect_entry};
use kf_abi::submit::MethodHeader;

/// Retry means no native publication/action was attempted. An uncertain write or native doorbell error is always
/// terminal; issued credits are never rolled back.
#[derive(Debug, PartialEq, Eq)]
pub enum Fail {
    /// Look again (a lock or admission was busy).
    Retry,
    /// Refused by name: the relay for this channel stops.
    Refused(String),
}
impl From<String> for Fail {
    fn from(v: String) -> Self {
        Self::Refused(v)
    }
}
impl From<&str> for Fail {
    fn from(v: &str) -> Self {
        Self::Refused(v.into())
    }
}
/// A fallible step.
pub type Turn<T> = Result<T, Fail>;

/// The refusal text of a partial-suffix trigger (counted with the scan refusals of a Fresh window).
pub const PARTIAL_SUFFIX: &str = "USER_RING_TRIGGER_PARTIAL_SUFFIX_UNSUPPORTED";

/// Where a turn is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Phase {
    /// Between turns.
    Idle,
    /// Observe the private `GP_GET` / `REFERENCE`.
    Obs,
    /// Write the guest's `GP_GET` back.
    Wb,
    /// A segment is in flight: the owner's segment machinery runs.
    Segment,
    /// Read `GP_PUT`.
    Put,
    /// Load the low dword of the entry.
    Lo,
    /// Load the high dword.
    Hi,
    /// Load the registry epoch (AFTER the entry).
    Epoch,
    /// Decide (validation, lifecycle plan).
    Plan,
    /// Publish the entry as it is (Pointer mode).
    PubPointer,
    /// The SYNC_WAIT gate.
    Gate,
    /// Publish a control entry (no pushbuffer).
    PubNop,
    /// Read the pushbuffer.
    PbRead,
    /// Scan it.
    ScanStep,
}

/// What a guest entry publication is (the trace text only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PubKind {
    /// Pointer mode: the entry as loaded.
    Pointer,
    /// A control entry (`LENGTH == 0`).
    Control,
}

/// The gate / marker / retry events the owner counts or reports.
#[derive(Clone, Debug, PartialEq)]
pub enum Ev {
    /// `GP_PUT` outside the ring.
    PutBound,
    /// `inspect_entry` refused the loaded value.
    EntryRefused,
    /// The scan refused a segment for good.
    ScanRefused,
    /// A SYNC_WAIT entry behind an open packet was read at once.
    CarriedUngated {
        /// The loaded entry.
        entry: u64,
        /// The cursor.
        cursor: u32,
        /// The ring generation.
        generation: u64,
        /// The packet open across the entry.
        carried: Option<(MethodHeader, usize)>,
    },
    /// A scan refusal is re-read and rescanned (the bounded `Grace`).
    Retry {
        /// The refusal.
        why: String,
        /// This is not the first retry of the same segment.
        was_retrying: bool,
        /// The cursor.
        cursor: u32,
    },
    /// The gate's barrier + marker were published.
    GatePublished,
    /// The gate was passed (its witness was processed).
    GateProceeded,
    /// The private ring had no credit.
    CreditWait,
}

/// The classification of a scanned segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Staged {
    /// The segment holds a registered trigger (the owner stages it).
    pub rare: bool,
}

/// A direct GP segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpSegment {
    /// The pushbuffer's GPU VA.
    pub gpu_va: u64,
    /// Its length in bytes.
    pub len_bytes: u64,
}

/// What the software-object check found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwView {
    /// The channel has a software object at all.
    pub have: bool,
    /// The object the relay holds is no longer live.
    pub gone: bool,
}

/// One publication the turn made (for the owner's measurement trace).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Published {
    /// A guest entry.
    Entry {
        /// Which path.
        kind: PubKind,
        /// The cursor before.
        cursor: u32,
        /// The entry.
        entry: u64,
    },
    /// The gate's barrier and marker.
    Gate {
        /// The marker count.
        count: u64,
    },
}

/// What a step tells its caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Call `step` again.
    More,
    /// The turn is over.
    Idle {
        /// Something was published.
        submitted: bool,
    },
    /// A segment is in flight: run the segment machinery.
    Segment,
    /// Shadow mode: the owner handles the loaded entry ([`Relay::raw`]).
    Shadow,
    /// The pushbuffer was read, scanned and classified: build the segment and run the segment machinery.
    Scanned {
        /// A registered trigger is inside.
        rare: bool,
    },
}

/// The I/O of a turn. `kf-qemu` implements it with the real accessors, the tests with simulated ones.
pub trait Io {
    /// The private ring's `GP_GET` and the tagged `REFERENCE`.
    fn hw_words(&mut self) -> Turn<(u32, u32)>;
    /// Write the guest's `GP_GET` back. `Ok(false)` = a lock was busy (look again).
    fn write_guest_get(&mut self, get: u32) -> Turn<bool>;
    /// The current source ring's entry count.
    fn src_entries(&mut self) -> u32;
    /// The entry-size budget `inspect_entry` enforces.
    fn max_entry_bytes(&mut self) -> u64;
    /// Read `GP_PUT`. `Ok(None)` = a walk is in progress (the wait is registered; nothing to do now).
    fn gp_put(&mut self) -> Turn<Option<u32>>;
    /// `GP_PUT != cursor`: a new entry is being loaded.
    fn begin_entry(&mut self, _put: u32) {}
    /// One dword of the source ring's slot. Production takes ONE 8-byte snapshot at the first call.
    fn load_dword(&mut self, slot: u32, hi: bool) -> Turn<u32>;
    /// The source address of a slot (diagnostic text only).
    fn source_va(&mut self, _slot: u32) -> u64 {
        0
    }
    /// Shadow (diagnostic) mode: the owner handles the loaded entry.
    fn is_shadow(&mut self) -> bool {
        false
    }
    /// The registry epoch. Production issues a SeqCst fence first (the entry load must precede it).
    fn registry_epoch(&mut self) -> u64;
    /// Live registrations of the channel's software object.
    fn registry_live(&mut self) -> u32;
    /// The software object (refreshed in Scan / Window).
    fn software(&mut self, refresh: bool) -> Turn<SwView>;
    /// Whether `value` names the channel's software object.
    fn sw_matches(&self, value: u32) -> bool;
    /// Read the entry's pushbuffer (the words and whether it came from video memory). `entry` is the loaded GP entry.
    fn read_pb(&mut self, entry: u64, seg: &GpSegment) -> Turn<(Vec<u32>, bool)>;
    /// Classify the words against the scan state (production: the software snapshot and `stage_plan`).
    fn stage(&mut self, scan: &Scan, words: &[u32]) -> Turn<Staged>;
    /// `inspect_entry` accepted `entry`: the value the relay tested (a test hook; the value stored must be this one).
    fn validated(&mut self, _entry: u64) {}
    /// Publish a guest entry (control entry or Pointer mode) behind the source guards.
    fn publish_entry(
        &mut self,
        kind: PubKind,
        fetch: &mut Fetch,
        generation: u64,
        guest_get: u32,
        entry: u64,
    ) -> Turn<()>;
    /// Publish the gate's barrier + tagged marker; returns the marker count.
    fn publish_gate(&mut self, fetch: &mut Fetch, marks: &mut Marks) -> Turn<u64>;
    /// Publish one batch marker.
    fn publish_marker(&mut self, fetch: &mut Fetch, marks: &mut Marks) -> Turn<()>;
    /// A counted / reported event.
    fn event(&mut self, _e: Ev) {}
}

/// The relay of one channel: the per-entry turn and the state it shares with the owner's segment machinery.
#[derive(Clone, Debug)]
pub struct Relay {
    /// The lifecycle machine.
    pub life: Lifecycle,
    /// The scan state (committed by the owner's segment machinery).
    pub scan: Scan,
    /// The SYNC_WAIT gate.
    pub sync: SyncGate,
    /// Markers issued / processed.
    pub marks: Marks,
    /// Private-ring fetch receipts.
    pub fetch: Fetch,
    /// The bounded retry of a scan refusal.
    pub grace: Grace,
    /// The next guest ring slot to read.
    pub cursor: u32,
    /// The guest ring generation.
    pub generation: u64,
    /// Guest entries relayed so far (monotone across ring generations).
    pub relayed: u64,
    /// Where the turn is.
    pub phase: Phase,
    /// A refusal ended the relay.
    pub dead: bool,
    /// More work is owed without a new doorbell.
    pub again: bool,
    /// What the turn ended needing.
    pub need: Need,
    /// Published guest work not yet followed by a batch marker.
    pub unnotified: bool,
    /// The most recent marker carries an interrupt.
    pub marker_event: bool,
    /// The marker count the outstanding gate waits for.
    pub gate_target: u64,
    /// The guest `GP_GET` write-back still owed.
    pub guest_get: Option<(u64, u32)>,
    /// The owner has a segment in flight (set before the turn).
    pub segment_pending: bool,
    /// Entries refused by `inspect_entry`.
    pub refused_entry: u64,
    /// Publications of the turn (every call of the Io's publish methods that succeeded).
    pub puts: u64,
    // the entry in flight
    /// The loaded entry.
    pub raw: u64,
    /// The `GP_PUT` value read this turn.
    pub put: u32,
    /// The packet the scan had open when this turn read `GP_PUT` (Scan / Window only): for the owner's
    /// `USER_RING_PUT_BOUNDARY_OPEN` line, valid in the turn that queued that notice.
    pub put_packet: Option<(MethodHeader, usize)>,
    /// The guest `GP_GET` last written back (what the guest sees; 0 at birth and after a source switch).
    pub written_get: u32,
    /// The guest `GP_GET` after this entry.
    pub next_get: u32,
    /// The decoded entry.
    pub desc: Option<Source>,
    /// The words of the segment read this turn.
    pub words: Vec<u32>,
    /// The turn's scan classification failed for good (trace hook for the owner).
    pub scan_failed: bool,
    /// The last publication of the turn (taken by the owner for its trace).
    pub published: Option<Published>,
    lo: u32,
    hi: u32,
    epoch: u64,
    put_abs: u64,
    /// The guest ring was full when this turn read `GP_PUT` (see [`Relay::written_get`]).
    pub put_full: bool,
    seed: bool,
    plan_forward: bool,
    permit: Option<ScanPermit>,
}

impl Relay {
    /// A relay at birth.
    ///
    /// # Errors
    /// The private ring capacity or allocation.
    pub fn new(priv_entries: u32, life: Lifecycle, scan: Scan) -> Result<Self, &'static str> {
        Ok(Self {
            life,
            scan,
            sync: SyncGate::default(),
            marks: Marks::default(),
            fetch: Fetch::new(priv_entries)?,
            grace: Grace::default(),
            cursor: 0,
            generation: 1,
            relayed: 0,
            phase: Phase::Idle,
            dead: false,
            again: false,
            need: Need::Idle,
            unnotified: false,
            marker_event: false,
            gate_target: 0,
            guest_get: None,
            segment_pending: false,
            refused_entry: 0,
            puts: 0,
            raw: 0,
            put: 0,
            put_packet: None,
            written_get: 0,
            next_get: 0,
            desc: None,
            words: Vec::new(),
            scan_failed: false,
            published: None,
            lo: 0,
            hi: 0,
            epoch: 0,
            put_abs: 0,
            put_full: false,
            seed: false,
            plan_forward: false,
            permit: None,
        })
    }

    /// Everything that decides behaviour, none of the counters (model checking).
    #[must_use]
    pub fn state_key(&self) -> String {
        format!(
            "{}|{:?}|{:?}|{:?}|{:?}|{:?}|{}|{}|{}|{:?}|{}|{}|{}|{}|{:?}|{}|{:?}|{}|{}|{}|{:x}|{}|{}|{}|{}|{}|{}|{}",
            self.life.state_key(self.relayed),
            self.scan,
            self.sync,
            self.marks,
            self.fetch,
            self.grace,
            self.cursor,
            self.generation,
            self.relayed,
            self.phase,
            self.dead,
            self.again,
            u8::from(self.unnotified),
            u8::from(self.marker_event),
            self.need,
            self.gate_target,
            self.guest_get,
            self.lo,
            self.hi,
            self.epoch,
            self.raw,
            self.put_abs.saturating_sub(self.relayed),
            u8::from(self.seed),
            u8::from(self.plan_forward),
            self.put,
            u8::from(self.segment_pending),
            self.written_get,
            u8::from(self.put_full),
        )
    }

    /// Start a turn.
    pub fn claim(&mut self) {
        self.need = Need::Idle;
        self.again = false;
        self.scan_failed = false;
        self.published = None;
        self.phase = Phase::Obs;
    }

    /// Hand the scan's SET_OBJECT tally to the lifecycle counters (the owner calls it at the end of every turn, after
    /// a scanned segment may have been committed).
    pub fn sync_tally(&mut self) {
        let t = self.scan.tally();
        self.life.sync_set_objects(t.software_binds, t.rebinds);
    }

    /// No turn is running.
    #[must_use]
    pub fn idle(&self) -> bool {
        self.phase == Phase::Idle
    }

    /// A guest entry was published on the private ring (any path): the monotone count and the lifecycle's per-mode
    /// bookkeeping (Pointer mode records a forwarded SYNC_WAIT entry).
    pub fn note_relayed(&mut self, entry: u64) {
        self.life.forwarded(self.relayed, entry);
        self.relayed = self.relayed.saturating_add(1);
    }

    /// The statistics' view of a publication (tests count it).
    fn count_put(&mut self) {
        self.puts += 1;
    }

    /// One micro-step. On an error the turn ends (the in-flight entry is dropped).
    ///
    /// # Errors
    /// A refusal or a retry request from the I/O or from the entry's own validation.
    pub fn step(&mut self, io: &mut impl Io) -> Turn<Step> {
        let r = self.step_inner(io);
        if let Err(e) = &r {
            self.phase = Phase::Idle;
            if matches!(e, Fail::Refused(_)) {
                self.dead = true;
            }
        }
        r
    }

    fn idle_step(&mut self, submitted: bool) -> Turn<Step> {
        self.phase = Phase::Idle;
        Ok(Step::Idle { submitted })
    }

    fn step_inner(&mut self, io: &mut impl Io) -> Turn<Step> {
        match self.phase {
            Phase::Idle => Ok(Step::Idle { submitted: false }),
            Phase::Obs => {
                let (get, reference) = io.hw_words()?;
                self.marks.observe(reference);
                for (generation, get) in self.fetch.observe(get).map_err(str::to_owned)? {
                    if generation == self.generation {
                        self.guest_get = Some((generation, get));
                    }
                }
                self.phase = Phase::Wb;
                Ok(Step::More)
            }
            Phase::Wb => {
                if let Some((generation, get)) = self.guest_get {
                    if generation == self.generation {
                        if io.write_guest_get(get)? {
                            self.written_get = get;
                            self.guest_get = None;
                        } else {
                            // an admission lock is busy: look again (spin, then the safety net)
                            self.need = Need::Contended;
                            return self.idle_step(false);
                        }
                    } else {
                        self.guest_get = None;
                    }
                }
                self.phase = if self.segment_pending {
                    Phase::Segment
                } else {
                    Phase::Put
                };
                Ok(Step::More)
            }
            Phase::Segment => {
                self.phase = Phase::Idle;
                Ok(Step::Segment)
            }
            Phase::Put => self.put_step(io),
            Phase::Lo => {
                self.lo = io.load_dword(self.cursor, false)?;
                self.phase = Phase::Hi;
                Ok(Step::More)
            }
            Phase::Hi => {
                self.hi = io.load_dword(self.cursor, true)?;
                self.raw = (u64::from(self.hi) << 32) | u64::from(self.lo);
                if io.is_shadow() {
                    self.phase = Phase::Idle;
                    return Ok(Step::Shadow);
                }
                self.phase = Phase::Epoch;
                Ok(Step::More)
            }
            Phase::Epoch => {
                self.epoch = io.registry_epoch();
                self.phase = Phase::Plan;
                Ok(Step::More)
            }
            Phase::Plan => self.plan_step(io),
            Phase::PubPointer => self.publish_guest(io, PubKind::Pointer),
            Phase::PubNop => self.publish_guest(io, PubKind::Control),
            Phase::Gate => self.gate_step(io),
            Phase::PbRead => {
                let Some(Source::Segment(d)) = self.desc else {
                    return Err("USER_RING_GP_FORMAT".into());
                };
                let permit = self.permit.take().ok_or("USER_RING_PERMIT_ABSENT")?;
                let (words, vram) = io.read_pb(
                    self.raw,
                    &GpSegment {
                        gpu_va: d.gpu_va,
                        len_bytes: d.len_bytes,
                    },
                )?;
                self.life.note_pb_read(&permit, words.len() * 4, vram);
                self.words = words;
                self.phase = Phase::ScanStep;
                Ok(Step::More)
            }
            Phase::ScanStep => self.scan_step(io),
        }
    }

    fn put_step(&mut self, io: &mut impl Io) -> Turn<Step> {
        let Some(put) = io.gp_put()? else {
            return self.idle_step(false);
        };
        let entries = io.src_entries();
        if put >= entries {
            io.event(Ev::PutBound);
            return Err("USER_RING_PUT_BOUND".into());
        }
        self.put = put;
        self.put_abs = self.relayed + u64::from(put.wrapping_sub(self.cursor) & (entries - 1));
        // the guest ring is FULL when the value it published leaves one slot before the `GP_GET` it sees
        self.put_full = put.wrapping_add(1) & (entries - 1) == self.written_get;
        let known = matches!(self.life.mode(), Mode::Scan | Mode::Window);
        self.put_packet = known.then(|| self.scan.carried()).flatten();
        let open = known.then(|| self.put_packet.is_some());
        self.life.observe_put(self.relayed, self.put_abs, open);
        if put == self.cursor {
            let submitted = self.end_batch(io)?;
            return self.idle_step(submitted);
        }
        io.begin_entry(put);
        self.phase = Phase::Lo;
        Ok(Step::More)
    }

    fn plan_step(&mut self, io: &mut impl Io) -> Turn<Step> {
        let entries = io.src_entries();
        let desc = match inspect_entry(self.raw, io.max_entry_bytes()) {
            Ok(d) => d,
            Err(e) => {
                io.event(Ev::EntryRefused);
                self.refused_entry += 1;
                let va = io.source_va(self.cursor);
                return Err(format!(
                    "USER_RING_GP_FORMAT {e:?} entry={:#018x} source_va={va:#x} cursor={:#x} put={:#x} entries={entries}",
                    self.raw, self.cursor, self.put
                )
                .into());
            }
        };
        io.validated(self.raw);
        self.desc = Some(desc);
        self.next_get = self.cursor.checked_add(1).ok_or("USER_RING_CURSOR")? & (entries - 1);
        let view = io.software(matches!(self.life.mode(), Mode::Scan | Mode::Window))?;
        if self.life.mode() == Mode::Pointer && view.gone {
            // the software object was freed while no pushbuffer was read: refused like the Scan path
            self.life.software_gone();
            return Err("USER_RING_SOFTWARE_OBJECT_GONE".into());
        }
        let sw_mask = self.scan.software_mask(|v| io.sw_matches(v));
        let obs = Obs {
            raw: self.raw,
            abs: self.relayed,
            epoch: self.epoch,
            put_abs: self.put_abs,
            put_full: self.put_full,
            sw_mask,
            have_software: view.have,
            quiescent: !self.sync.waiting(),
        };
        let plan = self.life.entry(&obs, || io.registry_live());
        match plan {
            Plan::Stop => self.idle_step(false),
            Plan::Forward => {
                self.plan_forward = true;
                self.phase = Phase::PubPointer;
                Ok(Step::More)
            }
            Plan::Scan {
                restart,
                seed_gate,
                permit,
            } => {
                if restart {
                    // The first entry of a Fresh window, at the anchor (a GP_PUT value): parser state restarted,
                    // bindings and sub-device mask kept; a stale gate proof from before the unscanned stretch is
                    // forgotten.
                    self.scan = self.scan.fresh_resume();
                    self.sync = SyncGate::default();
                    self.grace.clear();
                    self.life.restarted();
                }
                self.plan_forward = false;
                self.seed = seed_gate;
                self.permit = Some(permit);
                self.phase = Phase::Gate;
                Ok(Step::More)
            }
        }
    }

    /// The SYNC_WAIT gate (`GP_ENTRY1_SYNC_WAIT`): before touching anything at or after a SYNC_WAIT entry, author
    /// the same wait on the relay's own ring (barrier + tagged marker) and read on only once the marker has been
    /// processed. A window that follows Pointer mode with an unproven SYNC_WAIT behind it gates its first entry as
    /// if that entry were a SYNC_WAIT entry (`seed`). A SYNC_WAIT entry reached with a packet OPEN across it has no
    /// witness (no byte may enter an open packet) and is read at once and counted.
    fn gate_step(&mut self, io: &mut impl Io) -> Turn<Step> {
        let gate_entry = if self.seed {
            self.raw | GP_SYNC_WAIT
        } else {
            self.raw
        };
        let verdict = if gate_entry >> 63 & 1 == 1 && self.scan.carried().is_some() {
            io.event(Ev::CarriedUngated {
                entry: self.raw,
                cursor: self.cursor,
                generation: self.generation,
                carried: self.scan.carried(),
            });
            Gate::Proceed
        } else {
            self.sync
                .entry(gate_entry, self.generation, self.cursor, self.marks.seen())
        };
        match verdict {
            Gate::Wait => {
                // The marker's interrupt announces it (its processing writes the reference).
                self.need = Need::Marker {
                    target: self.gate_target,
                };
                self.idle_step(false)
            }
            Gate::PublishBarrier => {
                if self.fetch.available() < 2 {
                    self.credit_need(io)?;
                    return self.idle_step(false);
                }
                let count = io.publish_gate(&mut self.fetch, &mut self.marks)?;
                self.count_put();
                self.published = Some(Published::Gate { count });
                self.sync.published(count);
                self.gate_target = count;
                // the marker is this batch's marker too
                self.unnotified = false;
                self.marker_event = true;
                io.event(Ev::GatePublished);
                self.need = Need::Marker { target: count };
                self.idle_step(true)
            }
            Gate::Proceed => {
                if self.seed {
                    self.life.gate_proven();
                    self.seed = false;
                }
                io.event(Ev::GateProceeded);
                self.phase = match self.desc {
                    Some(Source::Nop) => Phase::PubNop,
                    _ => Phase::PbRead,
                };
                if self.phase == Phase::PubNop {
                    self.permit = None;
                }
                Ok(Step::More)
            }
        }
    }

    /// Publish the guest's entry as it was loaded and validated (Pointer mode or a control entry).
    fn publish_guest(&mut self, io: &mut impl Io, kind: PubKind) -> Turn<Step> {
        // one slot stays back for the batch marker
        if self.fetch.available() < 2 {
            self.credit_need(io)?;
            return self.idle_step(false);
        }
        io.publish_entry(
            kind,
            &mut self.fetch,
            self.generation,
            self.next_get,
            self.raw,
        )?;
        self.count_put();
        self.published = Some(Published::Entry {
            kind,
            cursor: self.cursor,
            entry: self.raw,
        });
        self.cursor = self.next_get;
        self.note_relayed(self.raw);
        self.unnotified = true;
        self.again = true;
        self.idle_step(true)
    }

    fn scan_step(&mut self, io: &mut impl Io) -> Turn<Step> {
        let was_retrying = self.grace.retrying();
        let planned = io.stage(&self.scan, &self.words);
        let counted = |why: &str| why.starts_with("USER_RING_SCAN ") || why == PARTIAL_SUFFIX;
        if let Err(Fail::Refused(why)) = &planned
            && counted(why)
            && !was_retrying
        {
            self.life.scan_refused();
        }
        let planned = match planned {
            // A scan refusal of a segment never published is re-read and rescanned `REFUSE_RETRIES` times -- a
            // COUNT, never a clock -- before it is final; the re-read is an immediate re-turn.
            Err(Fail::Refused(why))
                if counted(&why) && self.grace.refused(self.generation, self.cursor) =>
            {
                io.event(Ev::Retry {
                    why,
                    was_retrying,
                    cursor: self.cursor,
                });
                self.again = true;
                return self.idle_step(false);
            }
            other => other,
        };
        match planned {
            Ok(staged) => {
                self.grace.clear();
                self.phase = Phase::Idle;
                Ok(Step::Scanned { rare: staged.rare })
            }
            Err(e) => {
                // a refusal of the scan itself (stage_plan) is traced by the owner; a software-object error
                // (raised before the scan) is not
                self.scan_failed = matches!(&e, Fail::Refused(why) if why.starts_with("USER_RING_SCAN") || why == PARTIAL_SUFFIX);
                if matches!(e, Fail::Refused(_)) {
                    io.event(Ev::ScanRefused);
                }
                Err(e)
            }
        }
    }

    fn publish_marker(&mut self, io: &mut impl Io) -> Turn<()> {
        io.publish_marker(&mut self.fetch, &mut self.marks)?;
        self.count_put();
        self.unnotified = false;
        self.marker_event = true;
        self.life.marker_published();
        Ok(())
    }

    /// The turn found `GP_PUT == cursor`. End the batch: a marker behind unannounced work when a packet boundary
    /// allows it, and wait for that marker to be PROCESSED: its interrupt is the wake, its reference the proof, and
    /// processing implies the pops the guest's `GP_GET` write-back needs. With no marker possible the relay waits
    /// for the pops themselves (spin / safety net).
    ///
    /// # Errors
    /// The marker's publication.
    pub fn end_batch(&mut self, io: &mut impl Io) -> Turn<bool> {
        if self.unnotified {
            if !self
                .life
                .marker_possible(self.scan.carried().is_some(), self.relayed)
            {
                // no marker is possible (a packet is open, or Pointer mode is not at a GP_PUT boundary): the wake
                // for this work is the spin or the safety net
                self.unnotified = false;
                self.marker_event = false;
                self.need = Need::Popped {
                    target: self.fetch.issued(),
                };
                return Ok(false);
            }
            if self.fetch.available() >= 1 {
                self.publish_marker(io)?;
                self.need = Need::Marker {
                    target: self.marks.issued(),
                };
                return Ok(true);
            }
            self.need = Need::Popped {
                target: self.fetch.fetched() + 1,
            };
            return Ok(false);
        }
        if self.fetch.fetched() < self.fetch.issued() {
            // published work in flight: the last marker's processing frees the guest's ring (the `GP_GET`
            // write-back)
            self.need = if self.marker_event {
                Need::Marker {
                    target: self.marks.issued(),
                }
            } else {
                Need::Popped {
                    target: self.fetch.issued(),
                }
            };
        }
        Ok(false)
    }

    /// The private ring has no credit for what the turn wants to publish. Put a marker behind the unannounced work
    /// out of the reserved slot first, so SOME interrupt announces progress.
    ///
    /// # Errors
    /// The marker's publication.
    pub fn credit_need(&mut self, io: &mut impl Io) -> Turn<()> {
        let mut marker = self.marker_event;
        if self.unnotified {
            if self
                .life
                .marker_possible(self.scan.carried().is_some(), self.relayed)
                && self.fetch.available() >= 1
            {
                self.publish_marker(io)?;
                marker = true;
            } else {
                self.unnotified = false;
                self.marker_event = false;
                marker = false;
            }
        }
        io.event(Ev::CreditWait);
        self.need = if marker {
            Need::Marker {
                target: self.marks.issued(),
            }
        } else {
            Need::Popped {
                target: self.fetch.fetched() + 1,
            }
        };
        Ok(())
    }
}
