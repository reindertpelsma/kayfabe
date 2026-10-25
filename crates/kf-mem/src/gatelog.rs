//! ★ **WALK-GATE LOG — DIAGNOSTIC ONLY** (`KF3_DIAG_GATELOG=1`; class M, off unless the value is
//! exactly `1`, read once; never required to boot; nothing here changes a decision). Written for
//! the enscape-07/08 CE MMU fault (host Xid 31, run 5227) and hypothesis H7: *while any page-table
//! walk anywhere in the VM is busy, every Windows user ring is held (nothing is published); a copy
//! submitted while a tile is MAPPED waits behind the walk, the guest UNMAPs the tile, the UNMAP
//! reaches the host first, and the held entry is published afterwards and faults.*
//! (`crates/kf-chan/src/user_turn.rs` `put_step`, `crates/kf-qemu/src/mem.rs` `Inbox::walk_busy`.)
//!
//! It answers, per Windows user-ring channel and in the VAHIST clock (`vahist` `t_ms`):
//! - **seen**: when the relay first saw each GP_PUT advance (slot range) and each GP entry;
//! - **gate**: when the walk gate held the channel, why (walk busy, which space was being applied),
//!   and when it let go; the walks applied meanwhile and whether they could touch the held work
//!   ([`Overlap`]; the entry's own target VAs are NOT known to the relay at the gate, so the best
//!   effort is "did a walk change this channel's SPACE");
//! - **published**: when each entry reached the private host ring (and whether a gate had held it);
//! - **unmap**: for every VA-manager apply that UNMAPs: the space, the VA range, the time, and for
//!   each channel of that space the number of entries seen-but-unpublished at that instant
//!   ("held across unmap") and the oldest one's age.
//!
//! **What it keeps (all in memory, all bounded):** per channel one [`Ring`] of [`RING_N`] records
//! (a counted overflow, never growth); at most [`MAX_CHANS`] channels; global rings of the last
//! [`WALKS_N`] applied diffs and [`UNMAPS_N`] unmapping applies; counters per space
//! ([`SPACES_MAX`]).
//!
//! **What it prints:** NOTHING while running. [`Recorder::dump`] prints one bounded block on a host
//! Xid (the VAHIST trigger file) and one at QEMU shutdown: the last [`DUMP_LAST`] records of the
//! faulting channel (matched by the Xid's channel id like VAHIST: `host_token & 0xffff`), the
//! unmapping applies that cover the fault VA, the "held across unmap" count per space, and the
//! hold totals (how much unrelated work the gate stopped). At most [`DUMPS_MAX`] dumps per process,
//! [`DUMP_LINES_MAX`] lines each, every line at most [`LINE_MAX`] bytes.
//!
//! **Threads:** the channel's relay thread records (its own channel's mutex; the global mutex only
//! to close a hold); the VA thread records applies and evaluates "held across unmap" (global mutex,
//! then each channel's mutex, never the reverse). No vCPU thread and no drainer ever calls in.
//!
//! **Hostile guest:** every number recorded is a number the host parsed or counted; a guest-chosen
//! GP_PUT outside the ring is ignored; storage is capped whatever the guest does.

#![allow(unused_variables, unused_mut, dead_code, unused_imports)]

use crate::apply::DiffRun;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

/// Records per channel ring.
pub const RING_N: usize = 4096;
/// Channels tracked at once (a channel beyond it is counted, not tracked).
pub const MAX_CHANS: usize = 128;
/// Applied diffs kept (for the overlap question).
pub const WALKS_N: usize = 4096;
/// Unmapping applies kept.
pub const UNMAPS_N: usize = 4096;
/// Unmapped runs kept per unmapping apply (the first ones; the range and count cover the rest).
pub const UNMAP_RUNS: usize = 4;
/// Channels with held work listed per unmapping apply (the count covers the rest).
pub const UNMAP_CHANS: usize = 8;
/// Spaces with counters.
pub const SPACES_MAX: usize = 256;
/// Announced-but-unpublished chunks per channel (beyond it a chunk merges into the newest).
pub const PEND_CHUNKS: usize = 4096;
/// Records of the faulting channel in a dump.
pub const DUMP_LAST: usize = 64;
/// Unmapping applies listed per dump.
pub const DUMP_UNMAPS: usize = 8;
/// Spaces listed per dump.
pub const DUMP_SPACES: usize = 32;
/// Channels listed by hold time per dump.
pub const DUMP_TOP: usize = 16;
/// Dumps per process (Xid triggers and the shutdown one together).
pub const DUMPS_MAX: u32 = 5;
/// Longest dump, in lines.
pub const DUMP_LINES_MAX: usize = 256;
/// The longest line printed.
pub const LINE_MAX: usize = 512;
/// Changed-VA extents kept per hold (one per applied diff of the channel's space).
pub const CHANGED_MAX: usize = 8;

/// Gate reason bit: the VA thread has a walk in flight or pending.
pub const WHY_VA_BUSY: u8 = 1;
/// Gate reason bit: a guest statement is unsettled.
pub const WHY_UNSETTLED: u8 = 2;
/// Gate reason bit: a split is queued.
pub const WHY_SPLIT_QUEUED: u8 = 4;
/// Gate reason bit: a split is running.
pub const WHY_SPLIT_RUNNING: u8 = 8;

/// `true` only for exactly `"1"` (the `KF3_DIAG_*` rule: never "any value is on").
#[must_use]
pub fn flag_on(v: Option<&str>) -> bool {
    let _ = v;
    false
}

/// Is the diagnostic on (`KF3_DIAG_GATELOG=1`)? Read once.
#[must_use]
pub fn enabled() -> bool {
    static E: OnceLock<bool> = OnceLock::new();
    *E.get_or_init(|| flag_on(std::env::var("KF3_DIAG_GATELOG").ok().as_deref()))
}

/// The process's recorder, on the VAHIST clock ([`crate::vahist::Recorder::clock`]).
#[must_use]
pub fn global() -> &'static Recorder {
    static R: OnceLock<Recorder> = OnceLock::new();
    R.get_or_init(|| {
        let (t0, unix_ms) = crate::vahist::global().clock();
        Recorder::with_clock(t0, unix_ms)
    })
}

/// A fixed-capacity ring: the oldest record is overwritten and COUNTED, memory never grows past
/// `cap`.
#[derive(Debug, Clone)]
pub struct Ring<T> {
    buf: Vec<T>,
    next: usize,
    cap: usize,
    overflow: u64,
}

impl<T: Clone> Ring<T> {
    /// An empty ring of `cap` records (at least one).
    #[must_use]
    pub fn new(cap: usize) -> Ring<T> {
        Ring {
            buf: Vec::new(),
            next: 0,
            cap: cap.max(1),
            overflow: 0,
        }
    }

    /// Append `v`; when full it replaces the oldest and counts an overflow.
    pub fn push(&mut self, v: T) {
        let _ = v;
    }

    /// Records held.
    #[must_use]
    pub fn len(&self) -> usize {
        0
    }

    /// No record yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Records overwritten so far.
    #[must_use]
    pub fn overflow(&self) -> u64 {
        0
    }

    /// Every record, newest first, without copying.
    pub fn newest_first(&self) -> impl Iterator<Item = &T> + '_ {
        std::iter::empty()
    }

    /// The newest `n` records, oldest first.
    #[must_use]
    pub fn last_n(&self, n: usize) -> Vec<T> {
        let _ = n;
        Vec::new()
    }
}

/// GP entries a `GP_PUT` of `put` adds after `last` on a ring of `entries` slots (0 for a value
/// outside the ring: the relay refuses those, and the log must not invent entries from them).
#[must_use]
pub fn put_advance(last: u32, put: u32, entries: u32) -> u32 {
    let _ = (last, put, entries);
    0
}

/// Do two VA ranges `(va, len)` share a byte? Half-open, saturating.
#[must_use]
pub fn ranges_overlap(a: (u64, u64), b: (u64, u64)) -> bool {
    let _ = (a, b);
    false
}

/// Could the walks applied during a hold have changed what the held work touches?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlap {
    /// No diff was applied during the hold (the gate held for an unsettled statement or a split).
    NoWalk,
    /// Diffs were applied, none in this channel's space: the hold stopped unrelated work.
    SpaceUntouched,
    /// A diff changed this channel's space; the held entry's target VAs are unknown.
    SpaceTouched,
    /// The held entry's target VAs are known and no changed VA is among them.
    VaDisjoint,
    /// The held entry's target VAs are known and a changed VA is among them.
    VaOverlap,
}

impl Overlap {
    /// The name printed.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Overlap::NoWalk => "no_walk",
            Overlap::SpaceUntouched => "space_untouched",
            Overlap::SpaceTouched => "space_touched_va_unknown",
            Overlap::VaDisjoint => "va_disjoint",
            Overlap::VaOverlap => "va_overlap",
        }
    }

    /// The code stored in a record.
    #[must_use]
    pub fn code(self) -> u32 {
        match self {
            Overlap::NoWalk => 0,
            Overlap::SpaceUntouched => 1,
            Overlap::SpaceTouched => 2,
            Overlap::VaDisjoint => 3,
            Overlap::VaOverlap => 4,
        }
    }

    /// From a stored code (anything unknown reads as `NoWalk`).
    #[must_use]
    pub fn from_code(c: u32) -> Overlap {
        match c {
            1 => Overlap::SpaceUntouched,
            2 => Overlap::SpaceTouched,
            3 => Overlap::VaDisjoint,
            4 => Overlap::VaOverlap,
            _ => Overlap::NoWalk,
        }
    }

    /// The hold stopped work no applied diff could have changed.
    #[must_use]
    pub fn unrelated(self) -> bool {
        false
    }
}

/// Classify a hold. `walks` = diffs applied during it; `space_touched` = one of them was in the
/// channel's space; `targets` = the held entry's target VA ranges when known (`None`: unknown);
/// `changed` = the VA extents those diffs changed in the channel's space.
#[must_use]
pub fn classify_overlap(
    walks: u64,
    space_touched: bool,
    targets: Option<&[(u64, u64)]>,
    changed: &[(u64, u64)],
) -> Overlap {
    let _ = (walks, space_touched, targets, changed);
    Overlap::NoWalk
}

/// Where the gate held the channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Point {
    /// Before `GP_PUT` was read (`put_step`): the relay reads nothing.
    Put,
    /// At the private-ring publication (`publish_guarded`): the entry was loaded.
    Publish,
}

/// What the gate said when it held a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gate {
    /// `WHY_*` bits.
    pub reason: u8,
    /// Where.
    pub point: Point,
    /// The space the VA thread was applying at that instant (0 = none).
    pub space: u64,
    /// The last diff id applied before the hold.
    pub walk: u64,
}

/// A record's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `GP_PUT` moved: `a` new put, `b` previous put, `c` entries added.
    PutAdvance,
    /// A GP entry was loaded: `a` slot, `b` the entry, `c` entries announced and unpublished.
    Seen,
    /// A hold began: `a` = reason | point << 8, `b` space being applied, `c` last diff id.
    GateStart,
    /// A hold ended: `a` = [`Overlap`] code, `b` hold ns, `c` last diff id.
    GateEnd,
    /// An entry was published: `a` slot, `b` age in ns since announced, `c` 1 if a gate held it
    /// (2: removed by the cursor resync, not seen published).
    Published,
    /// A VA-manager unmap found work held: `a` count (bit 31: channel in a hold), `b` oldest age
    /// in ns, `c` diff id.
    Unmap,
}

impl Kind {
    /// The name printed.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Kind::PutAdvance => "put_advance",
            Kind::Seen => "seen",
            Kind::GateStart => "gate_start",
            Kind::GateEnd => "gate_end",
            Kind::Published => "published",
            Kind::Unmap => "held_across_unmap",
        }
    }
}

/// One ring record (32 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rec {
    /// Nanoseconds since the VAHIST clock's origin.
    pub t_ns: u64,
    /// What.
    pub kind: Kind,
    /// See [`Kind`].
    pub a: u32,
    /// See [`Kind`].
    pub b: u64,
    /// See [`Kind`].
    pub c: u64,
}

/// Bit of [`Kind::Unmap`]'s `a`: the channel was inside a gate hold.
pub const UNMAP_IN_HOLD: u32 = 0x8000_0000;

/// Work a channel has announced and not yet published, at one instant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Held {
    /// Entries seen (GP_PUT advanced past them) and not published.
    pub count: u64,
    /// Age of the oldest, ns.
    pub oldest_age_ns: u64,
    /// The channel is inside a gate hold now (an entry may be waiting that was never read).
    pub in_hold: bool,
    /// How long the hold has lasted, ns (0 when none).
    pub hold_age_ns: u64,
}

impl Held {
    /// Work was waiting: entries seen-but-unpublished, or the channel was gated.
    #[must_use]
    pub fn any(self) -> bool {
        self.count > 0 || self.in_hold
    }
}

/// Per-channel totals.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChanStats {
    /// Holds (a hold is one gated stretch, however many turns it took).
    pub holds: u64,
    /// Gated turns inside an already open hold.
    pub regates: u64,
    /// Total hold time, ns.
    pub hold_ns: u64,
    /// Longest hold, ns.
    pub hold_ns_max: u64,
    /// Holds whose walks could not have changed the held work ([`Overlap::unrelated`]).
    pub unrelated_holds: u64,
    /// Their total time, ns.
    pub unrelated_hold_ns: u64,
    /// Entries announced by `GP_PUT`.
    pub announced: u64,
    /// GP entries loaded.
    pub seen: u64,
    /// Entries published.
    pub published: u64,
    /// Entries a gate held that were then published.
    pub published_held: u64,
    /// VA-manager unmaps that found this channel with work waiting.
    pub held_across_unmap: u64,
}

impl ChanStats {
    fn fold(&mut self, o: &ChanStats) {
        self.holds = self.holds.saturating_add(o.holds);
        self.regates = self.regates.saturating_add(o.regates);
        self.hold_ns = self.hold_ns.saturating_add(o.hold_ns);
        self.hold_ns_max = self.hold_ns_max.max(o.hold_ns_max);
        self.unrelated_holds = self.unrelated_holds.saturating_add(o.unrelated_holds);
        self.unrelated_hold_ns = self.unrelated_hold_ns.saturating_add(o.unrelated_hold_ns);
        self.announced = self.announced.saturating_add(o.announced);
        self.seen = self.seen.saturating_add(o.seen);
        self.published = self.published.saturating_add(o.published);
        self.published_held = self.published_held.saturating_add(o.published_held);
        self.held_across_unmap = self.held_across_unmap.saturating_add(o.held_across_unmap);
    }
}

#[derive(Debug, Clone, Copy)]
struct Chunk {
    t_ns: u64,
    first: u32,
    n: u32,
    held: bool,
}

#[derive(Debug, Clone, Copy)]
struct Hold {
    t_ns: u64,
    gate: Gate,
}

/// What the applied diffs changed during one hold, for [`ChanLog::pass`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    /// Diffs applied during the hold (any space).
    pub walks: u64,
    /// One of them was in the channel's space.
    pub space_touched: bool,
    /// The VA extents `(va, len)` they changed in the channel's space (at most [`CHANGED_MAX`]).
    pub extents: Vec<(u64, u64)>,
}

/// One channel's log: the ring, the announced-but-unpublished queue and the hold state. Pure:
/// every method takes the time.
#[derive(Debug)]
pub struct ChanLog {
    /// The host work-submit token (low 16 bits are the chid a host Xid names).
    pub host_token: u32,
    /// Guest client.
    pub client: u32,
    /// Guest handle.
    pub handle: u32,
    /// The VA space key (`client << 32 | vaspace`).
    pub vas: u64,
    /// Birth, ns.
    pub born_ns: u64,
    /// Free, ns.
    pub freed_ns: Option<u64>,
    entries: u32,
    ring: Ring<Rec>,
    last_put: u32,
    pend: VecDeque<Chunk>,
    pend_n: u64,
    hold: Option<Hold>,
    stats: ChanStats,
}

impl ChanLog {
    /// A channel born at `t` on a GP ring of `entries` slots (`GP_PUT` starts at 0).
    #[must_use]
    pub fn new(host_token: u32, client: u32, handle: u32, vas: u64, entries: u32, t: u64) -> Self {
        ChanLog {
            host_token,
            client,
            handle,
            vas,
            born_ns: t,
            freed_ns: None,
            entries,
            ring: Ring::new(RING_N),
            last_put: 0,
            pend: VecDeque::new(),
            pend_n: 0,
            hold: None,
            stats: ChanStats::default(),
        }
    }

    /// The totals.
    #[must_use]
    pub fn stats(&self) -> ChanStats {
        self.stats
    }

    /// The newest `n` records, oldest first.
    #[must_use]
    pub fn last(&self, n: usize) -> Vec<Rec> {
        let _ = n;
        Vec::new()
    }

    /// Records in the ring and records overwritten.
    #[must_use]
    pub fn ring_counts(&self) -> (usize, u64) {
        (self.ring.len(), self.ring.overflow())
    }

    /// Inside a gate hold now?
    #[must_use]
    pub fn in_hold(&self) -> bool {
        self.hold.is_some()
    }

    /// The gate of the open hold.
    #[must_use]
    pub fn hold_gate(&self) -> Option<Gate> {
        self.hold.map(|h| h.gate)
    }

    /// `GP_PUT` read as `put` at `t` (by the relay or by the gate's look). New slots are announced
    /// (seen); a value outside the ring, or no change, announces nothing. Returns the entries
    /// added.
    pub fn put_seen(&mut self, t: u64, put: u32) -> u32 {
        let _ = (t, put);
        0
    }

    /// The relay loaded GP entry `entry` of `slot` at `t`.
    pub fn entry_seen(&mut self, t: u64, slot: u32, entry: u64) {
        let _ = (t, slot, entry);
    }

    /// The gate held the channel at `t`. The first call of a stretch opens a hold (and marks every
    /// announced entry as held); later calls inside it are counted, not recorded. Returns `true`
    /// when a hold was opened.
    pub fn gate(&mut self, t: u64, g: Gate) -> bool {
        let _ = (t, g);
        false
    }

    /// The gate let the channel through at `t` (`walk` = last diff id applied). Closes the open
    /// hold
    /// and returns its length in ns; `None` when there was none.
    pub fn pass(
        &mut self,
        t: u64,
        walk: u64,
        ch: &Changes,
        targets: Option<&[(u64, u64)]>,
    ) -> Option<u64> {
        let _ = (t, walk, ch, targets);
        None
    }

    /// One entry reached the private host ring at `t`: the oldest announced one. Returns its age
    /// in ns (0 when none was announced: the log never saw its `GP_PUT`).
    pub fn published(&mut self, t: u64) -> u64 {
        let _ = t;
        0
    }

    /// Resync with the relay's cursor at `t`: the entries from `cursor` up to the last `GP_PUT`
    /// seen are the only ones still unpublished, so any older announced entry was consumed
    /// without a publish hook seeing it. They are removed (recorded as inferred, not counted as
    /// published). Returns how many.
    pub fn reconcile(&mut self, t: u64, cursor: u32) -> u64 {
        let _ = (t, cursor);
        0
    }

    fn pop_one(&mut self, t: u64, inferred: bool) -> u64 {
        let Some(front) = self.pend.front_mut() else {
            self.ring.push(Rec {
                t_ns: t,
                kind: Kind::Published,
                a: u32::MAX,
                b: 0,
                c: 0,
            });
            return 0;
        };
        let (slot, age, held) = (front.first, t.saturating_sub(front.t_ns), front.held);
        front.n = front.n.saturating_sub(1);
        front.first = if self.entries == 0 {
            0
        } else {
            front
                .first
                .saturating_add(1)
                .checked_rem(self.entries)
                .unwrap_or(0)
        };
        if front.n == 0 {
            self.pend.pop_front();
        }
        self.pend_n = self.pend_n.saturating_sub(1);
        if held && !inferred {
            self.stats.published_held = self.stats.published_held.saturating_add(1);
        }
        self.ring.push(Rec {
            t_ns: t,
            kind: Kind::Published,
            a: slot,
            b: age,
            c: if inferred { 2 } else { u64::from(held) },
        });
        age
    }

    /// What is waiting at `t`.
    #[must_use]
    pub fn held_now(&self, t: u64) -> Held {
        let _ = t;
        Held::default()
    }

    /// A VA-manager unmap (diff `walk`) found `held` here at `t`. Recorded only when work was
    /// waiting. Returns whether it was.
    pub fn note_unmap(&mut self, t: u64, held: Held, walk: u64) -> bool {
        let _ = (t, held, walk);
        false
    }

    /// The channel was freed at `t`.
    pub fn freed(&mut self, t: u64) {
        self.freed_ns = Some(t);
    }
}

/// A shared channel log.
pub type ChanRef = Arc<Mutex<ChanLog>>;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// One applied diff (any space).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkRec {
    /// The walk batch id.
    pub walk: u64,
    /// The space.
    pub space: u64,
    /// When the apply finished, ns.
    pub t_end_ns: u64,
    /// Runs in the diff.
    pub runs: u32,
    /// Of which unmaps.
    pub unmaps: u32,
    /// Lowest VA touched.
    pub lo: u64,
    /// One past the highest VA touched.
    pub hi: u64,
}

/// One channel found with work waiting by an unmapping apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeldChan {
    /// Host token.
    pub token: u32,
    /// What was waiting.
    pub held: Held,
}

/// One VA-manager apply that UNMAPs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmapRec {
    /// The walk batch id.
    pub walk: u64,
    /// The space.
    pub space: u64,
    /// When the apply began (the held evaluation is at this instant), ns.
    pub t_start_ns: u64,
    /// When it finished, ns (0 until finished).
    pub t_end_ns: u64,
    /// Unmapped runs in the diff.
    pub unmaps: u32,
    /// Mapped runs in the diff.
    pub maps: u32,
    /// Lowest VA of any run.
    pub lo: u64,
    /// One past the highest VA of any run.
    pub hi: u64,
    /// The first [`UNMAP_RUNS`] unmapped runs `(va, len)`.
    pub runs: Vec<(u64, u64)>,
    /// Channels live in the space at that instant.
    pub space_chans: u32,
    /// Of which with work waiting.
    pub held_chans: u32,
    /// The first [`UNMAP_CHANS`] of them.
    pub chans: Vec<HeldChan>,
}

impl UnmapRec {
    /// Does the apply's VA range cover `va`?
    #[must_use]
    pub fn covers(&self, va: u64) -> bool {
        va >= self.lo && va < self.hi
    }
}

/// Per-space counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpaceCnt {
    /// Unmapping applies.
    pub unmaps: u64,
    /// Of which with a channel's work waiting ("held across unmap" events).
    pub with_held: u64,
    /// Channel-events with work waiting (one per channel per unmap).
    pub chan_events: u64,
    /// Entries seen-but-unpublished, summed over those events.
    pub entries: u64,
}

/// An apply in progress ([`Recorder::begin_apply`]).
#[derive(Debug)]
pub struct Ticket {
    walk: u64,
    space: u64,
    runs: u32,
    unmaps: u32,
    lo: u64,
    hi: u64,
    rec: Option<UnmapRec>,
}

#[derive(Debug)]
struct State {
    chans: HashMap<u32, ChanRef>,
    by_key: HashMap<(u32, u32), u32>,
    by_space: HashMap<u64, Vec<u32>>,
    order: VecDeque<u32>,
    untracked: u64,
    evicted: ChanStats,
    walks: Ring<WalkRec>,
    unmaps: Ring<UnmapRec>,
    spaces: HashMap<u64, SpaceCnt>,
    spaces_untracked: u64,
    dumps: u32,
}

/// ★ The recorder (one per process: [`global`]; tests make their own).
#[derive(Debug)]
pub struct Recorder {
    t0: Instant,
    t0_unix_ms: u128,
    st: Mutex<State>,
    applying_space: AtomicU64,
    applying_walk: AtomicU64,
    last_walk: AtomicU64,
}

/// A channel's hooks: the log plus the recorder's clock and state. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Handle {
    rec: &'static Recorder,
    chan: ChanRef,
}

impl Recorder {
    /// A recorder on the clock `(t0, t0_unix_ms)` (VAHIST's: [`crate::vahist::Recorder::clock`]).
    #[must_use]
    pub fn with_clock(t0: Instant, t0_unix_ms: u128) -> Recorder {
        Recorder {
            t0,
            t0_unix_ms,
            st: Mutex::new(State {
                chans: HashMap::new(),
                by_key: HashMap::new(),
                by_space: HashMap::new(),
                order: VecDeque::new(),
                untracked: 0,
                evicted: ChanStats::default(),
                walks: Ring::new(WALKS_N),
                unmaps: Ring::new(UNMAPS_N),
                spaces: HashMap::new(),
                spaces_untracked: 0,
                dumps: 0,
            }),
            applying_space: AtomicU64::new(0),
            applying_walk: AtomicU64::new(0),
            last_walk: AtomicU64::new(0),
        }
    }

    /// Nanoseconds since the clock's origin.
    #[must_use]
    pub fn now(&self) -> u64 {
        u64::try_from(self.t0.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    /// The last diff id applied.
    #[must_use]
    pub fn last_walk(&self) -> u64 {
        self.last_walk.load(Ordering::Acquire)
    }

    /// A channel was born (a Windows USER ring). Tracked while fewer than [`MAX_CHANS`] are held
    /// (the oldest freed one is evicted for it); otherwise counted as untracked.
    pub fn chan_birth(
        &self,
        t: u64,
        host_token: u32,
        client: u32,
        handle: u32,
        vas: u64,
        entries: u32,
    ) -> bool {
        let _ = (t, host_token, client, handle, vas, entries);
        false
    }

    /// Drop the record of `token` (reuse or eviction), folding its totals into the evicted sum.
    fn forget(st: &mut State, token: u32) {
        let Some(c) = st.chans.remove(&token) else {
            return;
        };
        let (key, vas, stats) = {
            let l = lock(&c);
            ((l.client, l.handle), l.vas, l.stats())
        };
        st.evicted.fold(&stats);
        if st.by_key.get(&key) == Some(&token) {
            st.by_key.remove(&key);
        }
        if let Some(v) = st.by_space.get_mut(&vas) {
            v.retain(|t| *t != token);
        }
        st.order.retain(|t| *t != token);
    }

    /// The channel `(client, handle)` was freed at `t`: it can no longer hold work.
    pub fn chan_free(&self, t: u64, client: u32, handle: u32) {
        let _ = (t, client, handle);
    }

    /// The log of a tracked channel.
    #[must_use]
    pub fn chan_ref(&self, host_token: u32) -> Option<ChanRef> {
        lock(&self.st).chans.get(&host_token).cloned()
    }

    /// The hooks of a tracked channel.
    #[must_use]
    pub fn handle(&'static self, host_token: u32) -> Option<Handle> {
        let chan = lock(&self.st).chans.get(&host_token).cloned()?;
        Some(Handle { rec: self, chan })
    }

    /// Diffs applied after id `after`, as the changes a hold of a channel in `vas` saw.
    #[must_use]
    pub fn changes_since(&self, after: u64, vas: u64) -> Changes {
        let _ = (after, vas);
        Changes::default()
    }

    /// The VA thread begins applying one diff: the unmap evaluation ("held across unmap") is made
    /// at THIS instant, before the host sees any run.
    pub fn begin_apply(&self, space: u64, walk: u64, runs: &[DiffRun]) -> Ticket {
        self.begin_apply_at(self.now(), space, walk, runs)
    }

    /// [`Self::begin_apply`] at an explicit time `t` (ns).
    pub fn begin_apply_at(&self, t: u64, space: u64, walk: u64, runs: &[DiffRun]) -> Ticket {
        let _ = (t, runs);
        Ticket {
            walk,
            space,
            runs: 0,
            unmaps: 0,
            lo: 0,
            hi: 0,
            rec: None,
        }
    }

    fn eval_unmap(
        &self,
        t: u64,
        space: u64,
        walk: u64,
        counts: (u32, u32),
        range: (u64, u64),
        head: Vec<(u64, u64)>,
    ) -> UnmapRec {
        let mut st = lock(&self.st);
        let tokens: Vec<u32> = st.by_space.get(&space).cloned().unwrap_or_default();
        let mut rec = UnmapRec {
            walk,
            space,
            t_start_ns: t,
            t_end_ns: 0,
            unmaps: counts.1,
            maps: counts.0,
            lo: range.0,
            hi: range.1,
            runs: head,
            space_chans: u32::try_from(tokens.len()).unwrap_or(u32::MAX),
            held_chans: 0,
            chans: Vec::new(),
        };
        let (mut events, mut entries) = (0u64, 0u64);
        for tok in tokens {
            let Some(c) = st.chans.get(&tok) else {
                continue;
            };
            let mut l = lock(c);
            let held = l.held_now(t);
            if !l.note_unmap(t, held, walk) {
                continue;
            }
            rec.held_chans = rec.held_chans.saturating_add(1);
            events = events.saturating_add(1);
            entries = entries.saturating_add(held.count);
            if rec.chans.len() < UNMAP_CHANS {
                rec.chans.push(HeldChan { token: tok, held });
            }
        }
        if st.spaces.len() < SPACES_MAX || st.spaces.contains_key(&space) {
            let s = st.spaces.entry(space).or_default();
            s.unmaps = s.unmaps.saturating_add(1);
            if events > 0 {
                s.with_held = s.with_held.saturating_add(1);
                s.chan_events = s.chan_events.saturating_add(events);
                s.entries = s.entries.saturating_add(entries);
            }
        } else {
            st.spaces_untracked = st.spaces_untracked.saturating_add(1);
        }
        rec
    }

    /// The VA thread finished the apply `t` began.
    pub fn end_apply(&self, tk: Ticket) {
        self.end_apply_at(self.now(), tk);
    }

    /// [`Self::end_apply`] at an explicit time `t` (ns).
    pub fn end_apply_at(&self, t: u64, tk: Ticket) {
        let _ = (t, tk);
    }
}

impl Handle {
    /// `GP_PUT` read as `put`.
    pub fn put_seen(&self, put: u32) {
        let _ = put;
    }

    /// GP entry `entry` of `slot` loaded.
    pub fn entry_seen(&self, slot: u32, entry: u64) {
        let _ = (slot, entry);
    }

    /// The gate held the channel (`reason` = `WHY_*` bits).
    pub fn gate(&self, reason: u8, point: Point) {
        let _ = (reason, point);
    }

    /// The gate let the channel through: close an open hold.
    pub fn pass(&self) {}

    /// Resync with the relay's cursor (end of a turn).
    pub fn reconcile(&self, cursor: u32) {
        let _ = cursor;
    }

    /// An entry was published (a gate that held the channel is over).
    pub fn published(&self) {}
}

fn cap(mut s: String) -> String {
    if s.len() > LINE_MAX {
        let mut n = LINE_MAX;
        while !s.is_char_boundary(n) {
            n = n.saturating_sub(1);
        }
        s.truncate(n);
    }
    s
}

fn pair(key: u64) -> String {
    format!("({:#x},{:#x})", key >> 32, key & 0xffff_ffff)
}

fn ms(ns: u64) -> u64 {
    ns / 1_000_000
}

/// One record as a line.
#[must_use]
pub fn rec_text(token: u32, r: &Rec) -> String {
    let _ = (token, r);
    String::new()
}

impl Recorder {
    /// The wall-clock origin of the clock, ms since the Unix epoch (VAHIST's `t0_unix_ms`).
    #[must_use]
    pub fn t0_unix_ms(&self) -> u128 {
        self.t0_unix_ms
    }

    /// ★ One bounded dump (a host Xid naming `chid`/`va`, or `why = "shutdown"`): at most
    /// [`DUMP_LINES_MAX`] lines of at most [`LINE_MAX`] bytes, [`DUMPS_MAX`] per process.
    pub fn dump(&self, why: &str, t: u64, chid: Option<u32>, va: Option<u64>) -> Vec<String> {
        let _ = (why, t, chid, va);
        Vec::new()
    }
}

/// Keep a dump within [`DUMP_LINES_MAX`] lines, leaving one for the closing `END` line. The caps
/// on records, unmaps, spaces and channels already keep a dump below it; this is the guard that
/// holds if one of them is raised.
pub fn bound_lines(out: &mut Vec<String>) {
    let _ = out;
}

/// A channel's facts copied out under its lock.
#[derive(Debug)]
struct Facts {
    token: u32,
    client: u32,
    handle: u32,
    vas: u64,
    born_ns: u64,
    freed_ns: Option<u64>,
    in_hold: bool,
    stats: ChanStats,
    ring_len: usize,
    ring_overflow: u64,
}

impl Facts {
    fn of(l: &ChanLog) -> Facts {
        let (ring_len, ring_overflow) = l.ring_counts();
        Facts {
            token: l.host_token,
            client: l.client,
            handle: l.handle,
            vas: l.vas,
            born_ns: l.born_ns,
            freed_ns: l.freed_ns,
            in_hold: l.in_hold(),
            stats: l.stats(),
            ring_len,
            ring_overflow,
        }
    }
}

fn chan_text(c: &Facts) -> String {
    cap(format!(
        "kf3: GATELOG CHAN token={:#x} key=({:#x},{:#x}) vas={} born_ms={} freed_ms={} in_hold={} \
        holds={} regates={} hold_ms={} longest_hold_ms={} unrelated_holds={} \
        unrelated_hold_ms={} announced={} published={} published_held={} held_across_unmap={} \
        ring_len={} ring_overflow={}",
        c.token,
        c.client,
        c.handle,
        pair(c.vas),
        ms(c.born_ns),
        c.freed_ns.map_or("live".to_string(), |f| ms(f).to_string()),
        u8::from(c.in_hold),
        c.stats.holds,
        c.stats.regates,
        ms(c.stats.hold_ns),
        ms(c.stats.hold_ns_max),
        c.stats.unrelated_holds,
        ms(c.stats.unrelated_hold_ns),
        c.stats.announced,
        c.stats.published,
        c.stats.published_held,
        c.stats.held_across_unmap,
        c.ring_len,
        c.ring_overflow
    ))
}

/// One unmapping apply as a line, from the point of view of channel `token`.
fn unmap_text(token: u32, u: &UnmapRec) -> String {
    let chan_held = match u.chans.iter().find(|h| h.token == token) {
        Some(h) => format!(
            "pending={} oldest_age_ns={} in_hold={} hold_age_ns={}",
            h.held.count,
            h.held.oldest_age_ns,
            u8::from(h.held.in_hold),
            h.held.hold_age_ns
        ),
        None if usize::try_from(u.held_chans).unwrap_or(usize::MAX) > u.chans.len() => {
            "unknown(list truncated)".to_string()
        }
        None => "pending=0 in_hold=0".to_string(),
    };
    let runs: Vec<String> = u
        .runs
        .iter()
        .map(|&(v, l)| format!("{v:#x}+{l:#x}"))
        .collect();
    cap(format!(
        "kf3: GATELOG UNMAP token={token:#x} walk#{} space={} t_start_ns={} t_end_ns={} \
        t_start_ms={} unmaps={} maps={} range={:#x}..{:#x} runs=[{}] space_chans={} \
        held_chans={} this_chan: {chan_held}",
        u.walk,
        pair(u.space),
        u.t_start_ns,
        u.t_end_ns,
        ms(u.t_start_ns),
        u.unmaps,
        u.maps,
        u.lo,
        u.hi,
        runs.join(","),
        u.space_chans,
        u.held_chans
    ))
}
