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

use crate::apply::DiffRun;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

pub mod ens3;
pub mod ens4;
pub mod ens4_packet;

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
    v == Some("1")
}

/// Is the diagnostic on (`KF3_DIAG_GATELOG=1`)? Read once.
#[must_use]
pub fn enabled() -> bool {
    static E: OnceLock<bool> = OnceLock::new();
    *E.get_or_init(|| {
        flag_on(std::env::var("KF3_DIAG_GATELOG").ok().as_deref()) || kf_host::diag::enabled()
    })
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
        if self.buf.len() < self.cap {
            self.buf.push(v);
            return;
        }
        if let Some(slot) = self.buf.get_mut(self.next) {
            *slot = v;
        }
        self.next = self.next.saturating_add(1);
        if self.next >= self.cap {
            self.next = 0;
        }
        self.overflow = self.overflow.saturating_add(1);
    }

    /// Records held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// No record yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Records overwritten so far.
    #[must_use]
    pub fn overflow(&self) -> u64 {
        self.overflow
    }

    /// Every record, newest first, without copying.
    pub fn newest_first(&self) -> impl Iterator<Item = &T> + '_ {
        let len = self.buf.len();
        (0..len).filter_map(move |k| {
            let back = len.saturating_sub(1).saturating_sub(k);
            let phys = self.next.saturating_add(back).checked_rem(len).unwrap_or(0);
            self.buf.get(phys)
        })
    }

    /// The newest `n` records, oldest first.
    #[must_use]
    pub fn last_n(&self, n: usize) -> Vec<T> {
        let len = self.buf.len();
        let take = n.min(len);
        let mut out = Vec::with_capacity(take);
        for i in len.saturating_sub(take)..len {
            let phys = self.next.saturating_add(i).checked_rem(len).unwrap_or(0);
            if let Some(v) = self.buf.get(phys) {
                out.push(v.clone());
            }
        }
        out
    }
}

/// GP entries a `GP_PUT` of `put` adds after `last` on a ring of `entries` slots (0 for a value
/// outside the ring: the relay refuses those, and the log must not invent entries from them).
#[must_use]
pub fn put_advance(last: u32, put: u32, entries: u32) -> u32 {
    if entries == 0 || put >= entries || last >= entries {
        return 0;
    }
    if put >= last {
        put.wrapping_sub(last)
    } else {
        entries.wrapping_sub(last).wrapping_add(put)
    }
}

/// Do two VA ranges `(va, len)` share a byte? Half-open, saturating.
#[must_use]
pub fn ranges_overlap(a: (u64, u64), b: (u64, u64)) -> bool {
    let (ae, be) = (a.0.saturating_add(a.1), b.0.saturating_add(b.1));
    a.1 != 0 && b.1 != 0 && a.0 < be && b.0 < ae
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
        matches!(
            self,
            Overlap::NoWalk | Overlap::SpaceUntouched | Overlap::VaDisjoint
        )
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
    if walks == 0 {
        return Overlap::NoWalk;
    }
    if !space_touched {
        return Overlap::SpaceUntouched;
    }
    let Some(targets) = targets else {
        return Overlap::SpaceTouched;
    };
    let hit = targets
        .iter()
        .any(|&t| changed.iter().any(|&c| ranges_overlap(t, c)));
    if hit {
        Overlap::VaOverlap
    } else {
        Overlap::VaDisjoint
    }
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
    /// Successful SET_REF: b absolute marker, c private descriptor PUT count.
    MarkerPublished,
    /// Validated ordinary observation: b absolute seen marker.
    MarkerSeen,
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
            Kind::MarkerPublished => "marker_published",
            Kind::MarkerSeen => "marker_seen",
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
    /// ENS4 observed gate-to-engine-idle latency, with missing/upper-bound coverage.
    pub exposure: ens4::Stats,
}

impl ChanStats {
    fn fold(&mut self, o: &ChanStats) {
        self.exposure.fold(o.exposure);
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
    progress: ens3::Progress,
    exposure: ens4::Exposure,
    packet: ens4_packet::Packet,
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
            progress: ens3::Progress::default(),
            exposure: ens4::Exposure::new(512),
            packet: ens4_packet::Packet::default(),
        }
    }

    /// The totals.
    #[must_use]
    pub fn stats(&self) -> ChanStats {
        let mut out = self.stats;
        out.exposure = self.exposure.stats();
        out
    }

    /// The newest `n` records, oldest first.
    #[must_use]
    pub fn last(&self, n: usize) -> Vec<Rec> {
        self.ring.last_n(n)
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
        let n = put_advance(self.last_put, put, self.entries);
        if n == 0 {
            return 0;
        }
        let prev = self.last_put;
        self.last_put = put;
        let held = self.hold.is_some();
        if self.pend.len() >= PEND_CHUNKS {
            if let Some(back) = self.pend.back_mut() {
                back.n = back.n.saturating_add(n);
                back.held |= held;
            }
        } else {
            self.pend.push_back(Chunk {
                t_ns: t,
                first: prev,
                n,
                held,
            });
        }
        self.pend_n = self.pend_n.saturating_add(u64::from(n));
        self.stats.announced = self.stats.announced.saturating_add(u64::from(n));
        self.ring.push(Rec {
            t_ns: t,
            kind: Kind::PutAdvance,
            a: put,
            b: u64::from(prev),
            c: u64::from(n),
        });
        n
    }

    /// The relay loaded GP entry `entry` of `slot` at `t`.
    pub fn entry_seen(&mut self, t: u64, slot: u32, entry: u64) {
        self.stats.seen = self.stats.seen.saturating_add(1);
        self.ring.push(Rec {
            t_ns: t,
            kind: Kind::Seen,
            a: slot,
            b: entry,
            c: self.pend_n,
        });
    }

    /// The gate held the channel at `t`. The first call of a stretch opens a hold (and marks every
    /// announced entry as held); later calls inside it are counted, not recorded. Returns `true`
    /// when a hold was opened.
    pub fn gate(&mut self, t: u64, g: Gate) -> bool {
        if self.hold.is_some() {
            self.stats.regates = self.stats.regates.saturating_add(1);
            return false;
        }
        self.hold = Some(Hold { t_ns: t, gate: g });
        self.exposure.gate(t);
        self.stats.holds = self.stats.holds.saturating_add(1);
        for c in &mut self.pend {
            c.held = true;
        }
        let point = match g.point {
            Point::Put => 0u32,
            Point::Publish => 1u32,
        };
        self.ring.push(Rec {
            t_ns: t,
            kind: Kind::GateStart,
            a: u32::from(g.reason) | (point << 8),
            b: g.space,
            c: g.walk,
        });
        true
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
        let h = self.hold.take()?;
        self.exposure.pass();
        let ns = t.saturating_sub(h.t_ns);
        let ov = classify_overlap(ch.walks, ch.space_touched, targets, &ch.extents);
        self.stats.hold_ns = self.stats.hold_ns.saturating_add(ns);
        self.stats.hold_ns_max = self.stats.hold_ns_max.max(ns);
        if ov.unrelated() {
            self.stats.unrelated_holds = self.stats.unrelated_holds.saturating_add(1);
            self.stats.unrelated_hold_ns = self.stats.unrelated_hold_ns.saturating_add(ns);
        }
        self.ring.push(Rec {
            t_ns: t,
            kind: Kind::GateEnd,
            a: ov.code(),
            b: ns,
            c: walk,
        });
        Some(ns)
    }

    /// One entry reached the private host ring at `t`: the oldest announced one. Returns its age
    /// in ns (0 when none was announced: the log never saw its `GP_PUT`).
    pub fn published(&mut self, t: u64) -> u64 {
        self.packet.published();
        self.stats.published = self.stats.published.saturating_add(1);
        self.pop_one(t, false)
    }

    /// Resync with the relay's cursor at `t`: the entries from `cursor` up to the last `GP_PUT`
    /// seen are the only ones still unpublished, so any older announced entry was consumed
    /// without a publish hook seeing it. They are removed (recorded as inferred, not counted as
    /// published). Returns how many.
    pub fn reconcile(&mut self, t: u64, cursor: u32) -> u64 {
        if self.entries == 0 || cursor >= self.entries {
            return 0;
        }
        let expect = u64::from(put_advance(cursor, self.last_put, self.entries));
        let mut n = 0u64;
        while self.pend_n > expect && !self.pend.is_empty() {
            self.pop_one(t, true);
            n = n.saturating_add(1);
        }
        if self.pend_n == 0 {
            self.exposure.idle();
        }
        n
    }

    fn pop_one(&mut self, t: u64, inferred: bool) -> u64 {
        let Some(front) = self.pend.front_mut() else {
            if !inferred {
                self.exposure.published(t, None, false);
            }
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
        if inferred {
            self.exposure.omitted();
        } else {
            self.exposure.published(t, Some(front.t_ns), held);
        }
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
        Held {
            count: self.pend_n,
            oldest_age_ns: self.pend.front().map_or(0, |c| t.saturating_sub(c.t_ns)),
            in_hold: self.hold.is_some(),
            hold_age_ns: self.hold.map_or(0, |h| t.saturating_sub(h.t_ns)),
        }
    }

    /// A VA-manager unmap (diff `walk`) found `held` here at `t`. Recorded only when work was
    /// waiting. Returns whether it was.
    pub fn note_unmap(&mut self, t: u64, held: Held, walk: u64) -> bool {
        if !held.any() {
            return false;
        }
        self.stats.held_across_unmap = self.stats.held_across_unmap.saturating_add(1);
        let count = u32::try_from(held.count)
            .unwrap_or(u32::MAX)
            .min(!UNMAP_IN_HOLD);
        self.ring.push(Rec {
            t_ns: t,
            kind: Kind::Unmap,
            a: count | if held.in_hold { UNMAP_IN_HOLD } else { 0 },
            b: held.oldest_age_ns,
            c: walk,
        });
        true
    }

    /// The channel was freed at `t`.
    pub fn freed(&mut self, t: u64) {
        self.exposure.retire();
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
    ens3: Mutex<ens3::History>,
    first_153_done: AtomicBool,
    t0: Instant,
    t0_unix_ms: u128,
    st: Mutex<State>,
    applying_space: AtomicU64,
    applying_walk: AtomicU64,
    last_walk: AtomicU64,
    shutdown_done: AtomicBool,
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
            ens3: Mutex::new(ens3::History::default()),
            first_153_done: AtomicBool::new(false),
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
            shutdown_done: AtomicBool::new(false),
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
        let mut st = lock(&self.st);
        Self::forget(&mut st, host_token);
        if st.chans.len() >= MAX_CHANS {
            let victim = st.order.iter().copied().find(|tok| {
                st.chans
                    .get(tok)
                    .is_some_and(|c| lock(c).freed_ns.is_some())
            });
            match victim {
                Some(tok) => Self::forget(&mut st, tok),
                None => {
                    st.untracked = st.untracked.saturating_add(1);
                    return false;
                }
            }
        }
        let c = ChanLog::new(host_token, client, handle, vas, entries, t);
        st.chans.insert(host_token, Arc::new(Mutex::new(c)));
        st.by_key.insert((client, handle), host_token);
        st.by_space.entry(vas).or_default().push(host_token);
        st.order.push_back(host_token);
        true
    }

    /// Drop the record of `token` (reuse or eviction), folding its totals into the evicted sum.
    fn forget(st: &mut State, token: u32) {
        let Some(c) = st.chans.remove(&token) else {
            return;
        };
        let (key, vas, stats) = {
            let mut l = lock(&c);
            l.exposure.retire();
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
        let mut st = lock(&self.st);
        let Some(&token) = st.by_key.get(&(client, handle)) else {
            return;
        };
        let Some(c) = st.chans.get(&token).cloned() else {
            return;
        };
        let vas = {
            let mut l = lock(&c);
            l.freed(t);
            l.vas
        };
        if let Some(v) = st.by_space.get_mut(&vas) {
            v.retain(|x| *x != token);
        }
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
        let st = lock(&self.st);
        let mut ch = Changes::default();
        for w in st.walks.newest_first() {
            if w.walk <= after {
                break;
            }
            ch.walks = ch.walks.saturating_add(1);
            if w.space == vas {
                ch.space_touched = true;
                if ch.extents.len() < CHANGED_MAX {
                    ch.extents.push((w.lo, w.hi.saturating_sub(w.lo)));
                }
            }
        }
        ch
    }

    /// The VA thread begins applying one diff: the unmap evaluation ("held across unmap") is made
    /// at THIS instant, before the host sees any run.
    pub fn begin_apply(&self, space: u64, walk: u64, runs: &[DiffRun]) -> Ticket {
        self.begin_apply_at(self.now(), space, walk, runs)
    }

    /// [`Self::begin_apply`] at an explicit time `t` (ns).
    pub fn begin_apply_at(&self, t: u64, space: u64, walk: u64, runs: &[DiffRun]) -> Ticket {
        self.applying_space.store(space, Ordering::Release);
        self.applying_walk.store(walk, Ordering::Release);
        let (mut lo, mut hi, mut unmaps, mut maps) = (u64::MAX, 0u64, 0u32, 0u32);
        let mut head: Vec<(u64, u64)> = Vec::new();
        for r in runs {
            lo = lo.min(r.va);
            hi = hi.max(r.va.saturating_add(r.len));
            if r.unmap {
                unmaps = unmaps.saturating_add(1);
                if head.len() < UNMAP_RUNS {
                    head.push((r.va, r.len));
                }
            } else {
                maps = maps.saturating_add(1);
            }
        }
        if lo == u64::MAX {
            lo = 0;
        }
        let rec =
            (unmaps > 0).then(|| self.eval_unmap(t, space, walk, (maps, unmaps), (lo, hi), head));
        Ticket {
            walk,
            space,
            runs: u32::try_from(runs.len()).unwrap_or(u32::MAX),
            unmaps,
            lo,
            hi,
            rec,
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
        let mut st = lock(&self.st);
        st.walks.push(WalkRec {
            walk: tk.walk,
            space: tk.space,
            t_end_ns: t,
            runs: tk.runs,
            unmaps: tk.unmaps,
            lo: tk.lo,
            hi: tk.hi,
        });
        if let Some(mut r) = tk.rec {
            r.t_end_ns = t;
            st.unmaps.push(r);
        }
        drop(st);
        self.last_walk.fetch_max(tk.walk, Ordering::AcqRel);
        self.applying_space.store(0, Ordering::Release);
        self.applying_walk.store(0, Ordering::Release);
    }
}

impl Handle {
    /// `GP_PUT` read as `put`.
    pub fn put_seen(&self, put: u32) {
        let t = self.rec.now();
        lock(&self.chan).put_seen(t, put);
    }

    /// GP entry `entry` of `slot` loaded.
    pub fn entry_seen(&self, slot: u32, entry: u64) {
        let t = self.rec.now();
        lock(&self.chan).entry_seen(t, slot, entry);
    }

    /// The gate held the channel (`reason` = `WHY_*` bits).
    pub fn gate(&self, reason: u8, point: Point) {
        let t = self.rec.now();
        let g = Gate {
            reason,
            point,
            space: self.rec.applying_space.load(Ordering::Acquire),
            walk: self.rec.last_walk(),
        };
        lock(&self.chan).gate(t, g);
    }

    /// The gate let the channel through: close an open hold.
    pub fn pass(&self) {
        let Some((walk0, vas)) = ({
            let l = lock(&self.chan);
            l.hold_gate().map(|g| (g.walk, l.vas))
        }) else {
            return;
        };
        let ch = self.rec.changes_since(walk0, vas);
        let (t, walk) = (self.rec.now(), self.rec.last_walk());
        lock(&self.chan).pass(t, walk, &ch, None);
    }

    /// Resync with the relay's cursor (end of a turn).
    pub fn reconcile(&self, cursor: u32) {
        let t = self.rec.now();
        lock(&self.chan).reconcile(t, cursor);
    }

    /// An entry was published (a gate that held the channel is over).
    pub fn published(&self) {
        self.pass();
        let t = self.rec.now();
        lock(&self.chan).published(t);
    }
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
    let body = match r.kind {
        Kind::PutAdvance => format!("put={} prev={} added={}", r.a, r.b, r.c),
        Kind::Seen => format!("slot={} entry={:#018x} pending={}", r.a, r.b, r.c),
        Kind::GateStart => format!(
            "why={:#x} point={} applying_space={} last_walk#={}",
            r.a & 0xff,
            if (r.a >> 8) & 1 == 0 {
                "put"
            } else {
                "publish"
            },
            if r.b == 0 {
                "none".to_string()
            } else {
                pair(r.b)
            },
            r.c
        ),
        Kind::GateEnd => format!(
            "overlap={} hold_ns={} last_walk#={}",
            Overlap::from_code(r.a).name(),
            r.b,
            r.c
        ),
        Kind::Published => format!(
            "slot={} age_ns={} held_by_gate={}",
            if r.a == u32::MAX {
                "unseen".to_string()
            } else {
                r.a.to_string()
            },
            r.b,
            r.c
        ),
        Kind::MarkerPublished | Kind::MarkerSeen => {
            format!("marker={} put={}", r.b, r.c)
        }
        Kind::Unmap => format!(
            "pending={} oldest_age_ns={} in_hold={} walk#={}",
            r.a & !UNMAP_IN_HOLD,
            r.b,
            u8::from(r.a & UNMAP_IN_HOLD != 0),
            r.c
        ),
    };
    cap(format!(
        "kf3: GATELOG REC token={token:#x} t_ns={} t_ms={} {} {body}",
        r.t_ns,
        ms(r.t_ns),
        r.kind.name()
    ))
}

impl Recorder {
    /// The wall-clock origin of the clock, ms since the Unix epoch (VAHIST's `t0_unix_ms`).
    #[must_use]
    pub fn t0_unix_ms(&self) -> u128 {
        self.t0_unix_ms
    }

    /// The shutdown dump, at most once per process whichever exit path asks first (the VA loop's
    /// end, or the process's `atexit` handler: QEMU leaves by `exit`, which never stops the loop).
    /// Empty when it was already printed.
    pub fn dump_shutdown(&self, t: u64) -> Vec<String> {
        if self.shutdown_done.swap(true, Ordering::AcqRel) {
            return Vec::new();
        }
        self.dump("shutdown", t, None, None)
    }

    /// ★ One bounded dump (a host Xid naming `chid`/`va`, or `why = "shutdown"`): at most
    /// [`DUMP_LINES_MAX`] lines of at most [`LINE_MAX`] bytes, [`DUMPS_MAX`] per process.
    pub fn dump(&self, why: &str, t: u64, chid: Option<u32>, va: Option<u64>) -> Vec<String> {
        let mut st = lock(&self.st);
        if st.dumps >= DUMPS_MAX {
            return vec![cap(format!(
                "kf3: GATELOG DUMP capped: {DUMPS_MAX} dump(s) already printed; why={why} not \
                dumped"
            ))];
        }
        st.dumps = st.dumps.saturating_add(1);
        let n = st.dumps;
        let chid_text = chid.map_or("none".to_string(), |c| format!("{c:#x}"));
        let va_text = va.map_or("none".to_string(), |v| format!("{v:#x}"));
        let mut facts: Vec<Facts> = st.chans.values().map(|c| Facts::of(&lock(c))).collect();
        let mut fleet = st.evicted;
        for f in &facts {
            fleet.fold(&f.stats);
        }
        let mut out = vec![cap(format!(
            "kf3: GATELOG DUMP #{n} why={why} t_ms={} t_ns={t} t0_unix_ms={} chid={chid_text} \
            va={va_text} chans={} untracked={} walks_kept={} walks_overflow={} unmaps_kept={} \
            unmaps_overflow={}",
            ms(t),
            self.t0_unix_ms,
            st.chans.len(),
            st.untracked,
            st.walks.len(),
            st.walks.overflow(),
            st.unmaps.len(),
            st.unmaps.overflow()
        ))];
        out.push(cap(format!(
            "kf3: GATELOG FLEET holds={} regates={} hold_ms={} unrelated_holds={} \
            unrelated_hold_ms={} longest_hold_ms={} announced={} published={} published_held={} \
            held_across_unmap={}",
            fleet.holds,
            fleet.regates,
            ms(fleet.hold_ns),
            fleet.unrelated_holds,
            ms(fleet.unrelated_hold_ns),
            ms(fleet.hold_ns_max),
            fleet.announced,
            fleet.published,
            fleet.published_held,
            fleet.held_across_unmap
        )));
        // 1. the faulting channel(s): newest first, matched by the Xid's channel id like VAHIST
        let mut hit: Vec<&Facts> = facts
            .iter()
            .filter(|f| chid.is_some_and(|x| f.token & 0xffff == x))
            .collect();
        hit.sort_by_key(|f| std::cmp::Reverse(f.born_ns));
        hit.truncate(2);
        for f in &hit {
            out.push(chan_text(f));
            if let Some(c) = st.chans.get(&f.token) {
                for r in lock(c).last(DUMP_LAST) {
                    out.push(rec_text(f.token, &r));
                }
            }
        }
        if chid.is_some() && hit.is_empty() {
            out.push(cap(format!(
                "kf3: GATELOG CHAN none tracked with chid={chid_text} (untracked={})",
                st.untracked
            )));
        }
        // 2. the unmapping applies of the faulting channel's space that cover the fault VA
        for f in &hit {
            let mut shown = 0usize;
            for u in st.unmaps.newest_first() {
                if shown >= DUMP_UNMAPS {
                    break;
                }
                if u.space != f.vas || !va.is_none_or(|v| u.covers(v)) {
                    continue;
                }
                shown = shown.saturating_add(1);
                out.push(unmap_text(f.token, u));
            }
        }
        // Preserve the per-boot endpoint in ordinary shutdown dumps too.
        out.push(cap(format!(
            "kf3: GATELOG ENS4_TOTAL {}",
            fleet.exposure.summary()
        )));
        // 3. "held across unmap" per space
        let mut spaces: Vec<(u64, SpaceCnt)> = st.spaces.iter().map(|(&k, &v)| (k, v)).collect();
        spaces.sort_by_key(|&(k, v)| (std::cmp::Reverse(v.with_held), k));
        let total = spaces
            .iter()
            .fold(0u64, |a, (_, v)| a.saturating_add(v.with_held));
        out.push(format!(
            "kf3: GATELOG SPACES spaces={} untracked={} held_across_unmap_total={total}",
            spaces.len(),
            st.spaces_untracked
        ));
        for (k, v) in spaces.iter().take(DUMP_SPACES) {
            out.push(cap(format!(
                "kf3: GATELOG SPACE {} unmaps={} held_across_unmap={} chan_events={} entries={}",
                pair(*k),
                v.unmaps,
                v.with_held,
                v.chan_events,
                v.entries
            )));
        }
        // 4. the channels the gate stopped longest
        facts.sort_by_key(|f| (std::cmp::Reverse(f.stats.hold_ns), f.token));
        for f in facts.iter().take(DUMP_TOP).filter(|f| f.stats.holds > 0) {
            out.push(cap(format!(
                "kf3: GATELOG HOLDTOP token={:#x} vas={} holds={} hold_ms={} unrelated_hold_ms={} \
                longest_hold_ms={}",
                f.token,
                pair(f.vas),
                f.stats.holds,
                ms(f.stats.hold_ns),
                ms(f.stats.unrelated_hold_ns),
                ms(f.stats.hold_ns_max)
            )));
        }
        bound_lines(&mut out);
        let lines = out.len().saturating_add(1);
        out.push(format!("kf3: GATELOG END #{n} lines={lines}"));
        out
    }
}

/// Keep a dump within [`DUMP_LINES_MAX`] lines, leaving one for the closing `END` line. The caps
/// on records, unmaps, spaces and channels already keep a dump below it; this is the guard that
/// holds if one of them is raised.
pub fn bound_lines(out: &mut Vec<String>) {
    out.truncate(DUMP_LINES_MAX.saturating_sub(1));
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
