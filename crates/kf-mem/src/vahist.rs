//! ★ **VA HISTORY + CHANNEL LIFETIMES — DIAGNOSTIC ONLY** (`KF3_DIAG_VAHIST=1`; class M, off
//! unless the value is exactly `1`, read once; never required to boot; nothing here changes a
//! decision). Written for the enscape-08 CE MMU fault (host Xid 31, `FAULT_PTE`
//! `ACCESS_TYPE_VIRT_WRITE`, 2026-10-24), where `KF3_MAPLOG` (about 1 MB/s) is too heavy to run.
//!
//! It answers ONE question after the fact, cheaply: *"the host faulted at VA X on host channel C:
//! which guest VA space was C's, and was X present, absent (refused / withheld), or recently
//! unmapped there — or was X a VA of the PREVIOUS owner of C's host id?"*
//!
//! **What it keeps (all in memory, all bounded):**
//! - per VA space (`VasKey`, at most [`MAX_SPACES`] live): its host space and whether it was a
//!   recycled spare; the last [`LAST_RUNS`] applied diff runs (every op, VA-contiguous runs of one
//!   walk coalesced, so a 4 144-run map is one entry); the last [`ABSENCE_EVENTS`] non-MAP events
//!   (UNMAP, UNMAP_REFUSED, HELD, UNKNOWN; coalesced the same way); the refused and the withheld
//!   (privileged) MAP runs, de-duplicated by `(va, len)` because the walker re-emits them on every
//!   walk ([`STICKY_MAX`] each); the walk refusals (a refused leaf has no VA in the report);
//! - the last [`RETIRED_KEPT`] retired spaces, with their row extents at retire
//!   ([`RETIRE_EXTENTS`] coalesced extents) — for the host-channel-reuse question;
//! - the last [`CHAN_RECORDS`] Windows USER-ring channel lifetimes (birth, classes, free).
//!
//! **What it prints:**
//! - `kf3: CHAN_LIFE birth|free #n …` — ONE line per channel birth and per free, never per entry,
//!   at most [`CHAN_LINES_MAX`] per process (births and frees share the cap);
//! - `kf3: VAHIST AUTO #n …` — ONE dump per space whose map was refused or whose walk refused
//!   leaves, at most [`AUTO_DUMPS`] per process (privileged withholding never fires it: every user
//!   space has privileged leaves, re-emitted every walk);
//! - `kf3: VAHIST LOOKUP #n …` … `kf3: VAHIST END #n` — on a trigger (the host's Xid text, or a
//!   `va=0x… chid=0x…` line, written to `KF3_DIAG_VAHIST_TRIGGER=<path>` by the host lane), at
//!   most [`MANUAL_DUMPS`] per process, each at most `1 + 8 + 8 × 10 + 64 + 1` lines.
//!   Every line is at most [`LINE_MAX`] bytes. Worst case for a whole boot: about 1 MB.
//!
//! **Threads:** the VA thread (`kf3-vamgr`) records applies, retires, polls the trigger file
//! (at most once per [`TRIGGER_POLL_MS`]) and answers lookups; the channel act thread records
//! births, classes and frees. One mutex, never taken by a vCPU thread, held only to copy or push
//! a few entries (the lookup copies under it and queries the live rows and the guest tables after
//! releasing it).
//!
//! **Hostile guest:** every value printed is a number the host parsed (VAs, lengths, handles,
//! codes); the trigger is host-written and read only up to [`TRIGGER_BYTES`]; the guest PTE probe
//! reads at most nine 8-byte words through the walker's store bounds check.
//!
//! **Verdicts** (one per candidate space): `PRESENT` (one of our rows covers X now), `UNMAPPED`
//! (the newest event covering X is an applied UNMAP), `REFUSED` (a MAP covering X was refused:
//! absent on the host), `WITHHELD` (a privileged MAP covering X was withheld from a user twin),
//! `HELD` (the host already held the VA), `UNMAP_REFUSED`, `ROW_GONE` (a MAP was recorded but no
//! row covers X now), `NOT_NAMED` (no recorded event covers X — read with `history_complete`),
//! and for a retired space `RETIRED_MAPPED` / `RETIRED_NOT_MAPPED` (its rows at retire).

use crate::apply::DiffRun;
use crate::refusal_dump::{aperture, decode, field, index_at, path_to};
use kf_cuda::abi::{
    KF_DIRS, KF_PS_NONE, KFWR_ACK_APPLIED, KFWR_ACK_FAILED, KFWR_ACK_HELD, KfFormat,
};
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Last applied runs kept per VA space (all ops, coalesced).
pub const LAST_RUNS: usize = 64;
/// Non-MAP events (UNMAP, UNMAP_REFUSED, HELD, UNKNOWN; coalesced) kept per VA space.
pub const ABSENCE_EVENTS: usize = 2048;
/// Refused and withheld MAP runs kept per VA space, each (de-duplicated by `(va, len)`).
pub const STICKY_MAX: usize = 256;
/// Live VA spaces tracked at once (a space beyond it is counted, not tracked).
pub const MAX_SPACES: usize = 256;
/// Retired VA spaces kept for the reuse question.
pub const RETIRED_KEPT: usize = 8;
/// Coalesced row extents snapshotted per retired space.
pub const RETIRE_EXTENTS: usize = 1024;
/// Channel lifetime records kept in memory.
pub const CHAN_RECORDS: usize = 512;
/// `CHAN_LIFE` lines printed per process (births and frees together).
pub const CHAN_LINES_MAX: usize = 4096;
/// Lookups answered per process (trigger file).
pub const MANUAL_DUMPS: u32 = 4;
/// Automatic dumps per process (one per space, on a refusal).
pub const AUTO_DUMPS: u32 = 16;
/// Bytes of a trigger looked at.
pub const TRIGGER_BYTES: usize = 512;
/// Minimum gap between two looks at the trigger file.
pub const TRIGGER_POLL_MS: u64 = 500;
/// The longest line printed.
pub const LINE_MAX: usize = 512;
/// Channels and candidate spaces per lookup; events per candidate.
const PER_LOOKUP: usize = 8;

/// `true` only for exactly `"1"` (the `KF3_DIAG_*` rule: never "any value is on").
#[must_use]
pub fn flag_on(v: Option<&str>) -> bool {
    v == Some("1")
}

/// Is the diagnostic on (`KF3_DIAG_VAHIST=1`)? Read once.
#[must_use]
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| flag_on(std::env::var("KF3_DIAG_VAHIST").ok().as_deref()))
}

/// `KF3_DIAG_VAHIST_TRIGGER`: the file the host lane writes the Xid text into (read once; `None`
/// when unset, empty or longer than 4096 bytes).
#[must_use]
pub fn trigger_path() -> Option<&'static Path> {
    static P: OnceLock<Option<PathBuf>> = OnceLock::new();
    P.get_or_init(|| {
        std::env::var_os("KF3_DIAG_VAHIST_TRIGGER")
            .filter(|v| !v.is_empty() && v.len() <= 4096)
            .map(PathBuf::from)
    })
    .as_deref()
}

/// The process's recorder.
#[must_use]
pub fn global() -> &'static Recorder {
    static R: OnceLock<Recorder> = OnceLock::new();
    R.get_or_init(Recorder::new)
}

/// A lookup request: the host's fault VA and, when given, its channel id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trigger {
    /// The faulting VA.
    pub va: u64,
    /// The host channel id the Xid named (`channel 0x…`).
    pub chid: Option<u32>,
}

/// Hex digits (and `_` separators) at the start of `s`; `None` for none or more than 64 bits.
fn hex_at(s: &str) -> Option<u64> {
    let mut v: u64 = 0;
    let mut n = 0u32;
    for c in s.chars() {
        if c == '_' {
            continue;
        }
        let Some(d) = c.to_digit(16) else { break };
        v = v.checked_mul(16)?.checked_add(u64::from(d))?;
        n = n.saturating_add(1);
    }
    (n > 0).then_some(v)
}

fn find_hex(s: &str, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|k| {
            s.find(k)
                .map(|i| s.get(i.saturating_add(k.len())..).unwrap_or(""))
        })
        .and_then(hex_at)
}

/// Parse a trigger: the host's NVRM Xid text (`channel 0x00000067 … faulted @ 0xb_5a225000`) or
/// `va=0x… [chid=0x…]`. Only the first [`TRIGGER_BYTES`] are looked at; the VA is required.
#[must_use]
pub fn parse_trigger(text: &[u8]) -> Option<Trigger> {
    let t = text.get(..text.len().min(TRIGGER_BYTES)).unwrap_or(&[]);
    let s = String::from_utf8_lossy(t).to_ascii_lowercase();
    let va = find_hex(&s, &["faulted @ 0x", "va=0x"])?;
    let chid = find_hex(&s, &["channel 0x", "chid=0x"]).and_then(|v| u32::try_from(v).ok());
    Some(Trigger { va, chid })
}

/// A Windows USER-ring channel birth, as the act thread knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChanBirth {
    /// Guest client.
    pub client: u32,
    /// Guest handle.
    pub handle: u32,
    /// Guest token index.
    pub guest_token: u32,
    /// Host work-submit token (on a GR runlist its low bits are the host chid an Xid names).
    pub host_token: u32,
    /// Guest process id, as declared.
    pub pid: Option<u32>,
    /// Engine.
    pub engine: u32,
    /// GP entries.
    pub gp_entries: u32,
    /// GPFIFO VA.
    pub gpfifo_va: u64,
    /// The VA space key (`client << 32 | vaspace`).
    pub vas: u64,
}

/// What a lookup may ask of the live VA plane (the VA thread implements it).
pub trait SpaceView {
    /// Keys of live spaces.
    fn live_keys(&self) -> Vec<u64>;
    /// Our row covering `va` in `key`: `(start, len, in guest RAM, backing offset)`.
    fn row_at(&self, key: u64, va: u64) -> Option<(u64, u64, bool, u64)>;
    /// The guest's PTE for `va` in `key`, printed ([`probe_pte`]); `None` when not readable.
    fn guest_pte(&self, key: u64, va: u64) -> Option<String>;
}

/// Ring sizes of one tracked space (tests, census).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpaceStats {
    /// Entries in the last-runs ring.
    pub last_len: usize,
    /// Entries in the absence ring.
    pub absent_len: usize,
    /// Absence entries evicted.
    pub absent_evicted: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Map,
    Unmap,
    Refused,
    UnmapRefused,
    Withheld,
    Held,
    Unknown,
    WalkRefused,
}

impl Op {
    fn name(self) -> &'static str {
        match self {
            Op::Map => "MAP",
            Op::Unmap => "UNMAP",
            Op::Refused => "REFUSED",
            Op::UnmapRefused => "UNMAP_REFUSED",
            Op::Withheld => "WITHHELD",
            Op::Held => "HELD",
            Op::Unknown => "UNKNOWN",
            Op::WalkRefused => "WALK_REFUSED",
        }
    }
}

/// One recorded event. `aux` is the ack code, or the refusal mask of a walk refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Ev {
    walk: u64,
    t_ms: u64,
    op: Op,
    va: u64,
    len: u64,
    ap: u8,
    kind: u8,
    aux: u32,
    /// Times seen (sticky entries: re-emitted every walk).
    count: u64,
    /// First walk / time seen (sticky entries).
    first_walk: u64,
    first_ms: u64,
}

impl Ev {
    fn covers(&self, va: u64) -> bool {
        va.checked_sub(self.va).is_some_and(|d| d < self.len)
    }
}

/// Push `e`, extending the newest entry when it is the same op of the same walk and VA-adjacent.
/// Returns whether an entry was evicted.
fn push_coalesced(ring: &mut VecDeque<Ev>, cap: usize, e: Ev) -> bool {
    if let Some(b) = ring.back_mut()
        && b.op == e.op
        && b.walk == e.walk
        && b.ap == e.ap
        && b.kind == e.kind
        && b.va.checked_add(b.len) == Some(e.va)
    {
        b.len = b.len.saturating_add(e.len);
        return false;
    }
    ring.push_back(e);
    if ring.len() > cap {
        ring.pop_front();
        return true;
    }
    false
}

/// Record a refused/withheld run once per `(va, len)`; returns whether it is new.
fn push_sticky(ring: &mut VecDeque<Ev>, e: Ev, evicted: &mut u64) -> bool {
    if let Some(i) = ring.iter().position(|s| s.va == e.va && s.len == e.len) {
        if let Some(mut s) = ring.remove(i) {
            s.count = s.count.saturating_add(1);
            s.walk = e.walk;
            s.t_ms = e.t_ms;
            ring.push_back(s);
        }
        return false;
    }
    ring.push_back(e);
    if ring.len() > STICKY_MAX {
        ring.pop_front();
        *evicted = evicted.saturating_add(1);
    }
    true
}

#[derive(Debug, Clone, Default)]
struct Space {
    host_space: u64,
    recycled: bool,
    born_ms: u64,
    last: VecDeque<Ev>,
    absent: VecDeque<Ev>,
    absent_evicted: u64,
    refused: VecDeque<Ev>,
    withheld: VecDeque<Ev>,
    sticky_evicted: u64,
    walk_refusals: u64,
    last_refusal: Option<Ev>,
    runs: u64,
}

impl Space {
    /// Every recorded event covering `va`, newest first, at most `n`.
    fn covering(&self, va: u64, n: usize) -> Vec<Ev> {
        let mut v: Vec<Ev> = self
            .last
            .iter()
            .chain(self.absent.iter())
            .chain(self.refused.iter())
            .chain(self.withheld.iter())
            .filter(|e| e.covers(va))
            .copied()
            .collect();
        // Newest first; a duplicate (an event both in `last` and in a ring) shows once.
        v.sort_by_key(|e| std::cmp::Reverse((e.walk, e.t_ms)));
        v.dedup();
        v.truncate(n);
        v
    }
}

#[derive(Debug, Clone)]
struct Retired {
    key: u64,
    retired_ms: u64,
    space: Space,
    extents: Vec<(u64, u64)>,
    extents_truncated: bool,
    live_at_retire: u64,
}

#[derive(Debug, Clone)]
struct ChanLife {
    b: ChanBirth,
    born_ms: u64,
    freed_ms: Option<u64>,
    ok: Option<bool>,
    classes: Vec<u32>,
}

#[derive(Debug)]
struct State {
    t0: Instant,
    t0_unix_ms: u128,
    spaces: std::collections::HashMap<u64, Space>,
    spaces_untracked: u64,
    retired: VecDeque<Retired>,
    chans: VecDeque<ChanLife>,
    chan_lines: usize,
    chan_lines_dropped: u64,
    manual_dumps: u32,
    auto_dumps: u32,
    auto_keys: Vec<u64>,
    trig_polled: Option<Instant>,
    trig_last: Option<Vec<u8>>,
}

/// ★ The recorder (one per process: [`global`]; tests make their own).
#[derive(Debug)]
pub struct Recorder {
    st: Mutex<State>,
}

impl Default for Recorder {
    fn default() -> Recorder {
        Recorder::new()
    }
}

fn pair(key: u64) -> String {
    format!("({:#x},{:#x})", key >> 32, key & 0xffff_ffff)
}

/// Truncate to [`LINE_MAX`] bytes (all our text is ASCII).
fn cap(mut s: String) -> String {
    if s.len() > LINE_MAX {
        let mut at = LINE_MAX.saturating_sub(3);
        while !s.is_char_boundary(at) {
            at = at.saturating_sub(1);
        }
        s.truncate(at);
        s.push_str("...");
    }
    s
}

fn ev_text(e: &Ev) -> String {
    let aux = if e.op == Op::WalkRefused {
        format!("mask={:#x}", e.aux)
    } else {
        format!("ack={}", e.aux)
    };
    let seen = if e.count > 1 {
        format!(
            " seen={} first_walk#{} first_ms={}",
            e.count, e.first_walk, e.first_ms
        )
    } else {
        String::new()
    };
    format!(
        "walk#{} t_ms={} op={} va={:#x} len={:#x} ap={} kind={:#x} {aux}{seen}",
        e.walk,
        e.t_ms,
        e.op.name(),
        e.va,
        e.len,
        e.ap,
        e.kind
    )
}

/// One candidate space of a lookup, copied out under the lock.
struct Cand {
    key: u64,
    role: &'static str,
    space: Option<Space>,
    retired: Option<(u64, bool, bool, u64)>, // (retired_ms, extents cover X, truncated, live)
}

impl Recorder {
    /// A fresh recorder (its clock starts now).
    #[must_use]
    pub fn new() -> Recorder {
        Recorder {
            st: Mutex::new(State {
                t0: Instant::now(),
                t0_unix_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis()),
                spaces: std::collections::HashMap::new(),
                spaces_untracked: 0,
                retired: VecDeque::new(),
                chans: VecDeque::new(),
                chan_lines: 0,
                chan_lines_dropped: 0,
                manual_dumps: 0,
                auto_dumps: 0,
                auto_keys: Vec::new(),
                trig_polled: None,
                trig_last: None,
            }),
        }
    }

    fn st(&self) -> std::sync::MutexGuard<'_, State> {
        self.st
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn ms(st: &State) -> u64 {
        u64::try_from(st.t0.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// A space got its host space (a fresh one, or a recycled spare). Replaces any record of the
    /// same key.
    pub fn note_space(&self, key: u64, host_space: u64, recycled: bool) {
        let mut st = self.st();
        if st.spaces.len() >= MAX_SPACES && !st.spaces.contains_key(&key) {
            st.spaces_untracked = st.spaces_untracked.saturating_add(1);
            return;
        }
        let born_ms = Self::ms(&st);
        st.spaces.insert(
            key,
            Space {
                host_space,
                recycled,
                born_ms,
                ..Space::default()
            },
        );
    }

    /// ★ One space's apply: its diff runs and their verdicts (`KFWR_ACK_*`), whether its target
    /// withholds privileged leaves, and the walk's refusal mask for it. Returns `true` when a MAP
    /// was refused for the first time or the walk refused leaves (the caller's auto dump).
    pub fn record_apply(
        &self,
        key: u64,
        walk: u64,
        runs: &[DiffRun],
        codes: &[u8],
        withholds_priv: bool,
        walk_refused_mask: u32,
    ) -> bool {
        let mut st = self.st();
        let t_ms = Self::ms(&st);
        if !st.spaces.contains_key(&key) {
            if st.spaces.len() >= MAX_SPACES {
                st.spaces_untracked = st.spaces_untracked.saturating_add(1);
                return false;
            }
            st.spaces.insert(
                key,
                Space {
                    born_ms: t_ms,
                    ..Space::default()
                },
            );
        }
        let Some(sp) = st.spaces.get_mut(&key) else {
            return false;
        };
        let mut fresh_refusal = false;
        for (i, r) in runs.iter().enumerate() {
            let code = codes.get(i).copied();
            let op = match (r.unmap, code) {
                (true, Some(KFWR_ACK_APPLIED | KFWR_ACK_HELD)) => Op::Unmap,
                (true, Some(KFWR_ACK_FAILED)) => Op::UnmapRefused,
                (false, Some(KFWR_ACK_APPLIED)) => Op::Map,
                (false, Some(KFWR_ACK_HELD)) => Op::Held,
                (false, Some(KFWR_ACK_FAILED)) if r.privileged && withholds_priv => Op::Withheld,
                (false, Some(KFWR_ACK_FAILED)) => Op::Refused,
                _ => Op::Unknown,
            };
            let e = Ev {
                walk,
                t_ms,
                op,
                va: r.va,
                len: r.len,
                ap: r.ap,
                kind: r.kind,
                aux: u32::from(code.unwrap_or(0xff)),
                count: 1,
                first_walk: walk,
                first_ms: t_ms,
            };
            sp.runs = sp.runs.saturating_add(1);
            match op {
                Op::Refused => {
                    fresh_refusal |= push_sticky(&mut sp.refused, e, &mut sp.sticky_evicted);
                }
                Op::Withheld => {
                    push_sticky(&mut sp.withheld, e, &mut sp.sticky_evicted);
                }
                Op::Map | Op::WalkRefused => {}
                Op::Unmap | Op::UnmapRefused | Op::Held | Op::Unknown => {
                    if push_coalesced(&mut sp.absent, ABSENCE_EVENTS, e) {
                        sp.absent_evicted = sp.absent_evicted.saturating_add(1);
                    }
                }
            }
            push_coalesced(&mut sp.last, LAST_RUNS, e);
        }
        if walk_refused_mask != 0 {
            let e = Ev {
                walk,
                t_ms,
                op: Op::WalkRefused,
                va: 0,
                len: 0,
                ap: 0,
                kind: 0,
                aux: walk_refused_mask,
                count: 1,
                first_walk: walk,
                first_ms: t_ms,
            };
            sp.walk_refusals = sp.walk_refusals.saturating_add(1);
            sp.last_refusal = Some(e);
            push_coalesced(&mut sp.last, LAST_RUNS, e);
        }
        fresh_refusal || walk_refused_mask != 0
    }

    /// Ring sizes of `key` (tests, census).
    #[must_use]
    pub fn space_stats(&self, key: u64) -> Option<SpaceStats> {
        let st = self.st();
        st.spaces.get(&key).map(|s| SpaceStats {
            last_len: s.last.len(),
            absent_len: s.absent.len(),
            absent_evicted: s.absent_evicted,
        })
    }

    /// ★ An automatic dump of `key`'s history (once per space, at most [`AUTO_DUMPS`] per
    /// process; empty otherwise). `why` is the caller's own text.
    #[must_use]
    pub fn auto_dump(&self, key: u64, why: &str) -> Vec<String> {
        let mut st = self.st();
        if st.auto_dumps >= AUTO_DUMPS || st.auto_keys.contains(&key) {
            return Vec::new();
        }
        let Some(sp) = st.spaces.get(&key).cloned() else {
            return Vec::new();
        };
        st.auto_dumps = st.auto_dumps.saturating_add(1);
        st.auto_keys.push(key);
        let n = st.auto_dumps;
        let t_ms = Self::ms(&st);
        let t0 = st.t0_unix_ms;
        drop(st);
        let why: String = why
            .chars()
            .filter(|c| c.is_ascii_graphic() || *c == ' ')
            .take(64)
            .collect();
        let mut out = vec![cap(format!(
            "kf3: VAHIST AUTO #{n} key={} why={why} t_ms={t_ms} t0_unix_ms={t0} host_space={:#x} recycled={} born_ms={} runs={} walk_refusals={} refused={} withheld={}",
            pair(key),
            sp.host_space,
            sp.recycled,
            sp.born_ms,
            sp.runs,
            sp.walk_refusals,
            sp.refused.len(),
            sp.withheld.len()
        ))];
        for e in sp.refused.iter().rev().take(PER_LOOKUP) {
            out.push(cap(format!(
                "kf3: VAHIST EV key={} {}",
                pair(key),
                ev_text(e)
            )));
        }
        if let Some(e) = &sp.last_refusal {
            out.push(cap(format!(
                "kf3: VAHIST EV key={} {}",
                pair(key),
                ev_text(e)
            )));
        }
        let len = sp.last.len();
        for (i, e) in sp.last.iter().enumerate() {
            out.push(cap(format!(
                "kf3: VAHIST LAST key={} {}/{len} {}",
                pair(key),
                i.saturating_add(1),
                ev_text(e)
            )));
        }
        let lines = out.len().saturating_add(1);
        out.push(format!("kf3: VAHIST END #{n} auto lines={lines}"));
        out
    }

    /// ★ A space retired: its record moves to the retired ring with its row extents (VA-sorted
    /// `(va, len)`; coalesced, at most [`RETIRE_EXTENTS`]) and its live-channel count.
    pub fn retire(&self, key: u64, extents: Vec<(u64, u64)>, live: u64) {
        let mut merged: Vec<(u64, u64)> = Vec::new();
        let mut truncated = false;
        for (va, len) in extents {
            if let Some(last) = merged.last_mut()
                && last.0.checked_add(last.1) == Some(va)
            {
                last.1 = last.1.saturating_add(len);
                continue;
            }
            if merged.len() >= RETIRE_EXTENTS {
                truncated = true;
                break;
            }
            merged.push((va, len));
        }
        let mut st = self.st();
        let retired_ms = Self::ms(&st);
        let space = st.spaces.remove(&key).unwrap_or_default();
        st.retired.push_back(Retired {
            key,
            retired_ms,
            space,
            extents: merged,
            extents_truncated: truncated,
            live_at_retire: live,
        });
        if st.retired.len() > RETIRED_KEPT {
            st.retired.pop_front();
        }
    }

    fn chan_line(st: &mut State, text: String) -> Option<String> {
        if st.chan_lines >= CHAN_LINES_MAX {
            st.chan_lines_dropped = st.chan_lines_dropped.saturating_add(1);
            return None;
        }
        st.chan_lines = st.chan_lines.saturating_add(1);
        Some(cap(text.replacen("#N", &format!("#{}", st.chan_lines), 1)))
    }

    /// ★ A Windows USER-ring channel was born: recorded, and its ONE line (`None` past the cap).
    #[must_use]
    pub fn chan_birth(&self, b: ChanBirth) -> Option<String> {
        let mut st = self.st();
        let born_ms = Self::ms(&st);
        let text = format!(
            "kf3: CHAN_LIFE birth #N t_ms={born_ms} key=({:#x},{:#x}) guest_tok={:#x} host_token={:#x} pid={} engine={:#x} gp={} gpfifo={:#x} vas={}",
            b.client,
            b.handle,
            b.guest_token,
            b.host_token,
            b.pid.map_or("none".to_string(), |p| format!("{p:#x}")),
            b.engine,
            b.gp_entries,
            b.gpfifo_va,
            pair(b.vas)
        );
        st.chans.push_back(ChanLife {
            b,
            born_ms,
            freed_ms: None,
            ok: None,
            classes: Vec::new(),
        });
        if st.chans.len() > CHAN_RECORDS {
            st.chans.pop_front();
        }
        Self::chan_line(&mut st, text)
    }

    /// An engine object of `class` was kept on the live channel `(client, handle)` (at most 8
    /// distinct classes are kept).
    pub fn chan_class(&self, client: u32, handle: u32, class: u32) {
        let mut st = self.st();
        if let Some(c) = st
            .chans
            .iter_mut()
            .rev()
            .find(|c| c.b.client == client && c.b.handle == handle && c.freed_ms.is_none())
            && !c.classes.contains(&class)
            && c.classes.len() < 8
        {
            c.classes.push(class);
        }
    }

    /// ★ The live channel `(client, handle)` was freed (`ok`: the host free succeeded): its ONE
    /// line, or `None` (never recorded, already freed, or past the cap).
    #[must_use]
    pub fn chan_free(&self, client: u32, handle: u32, ok: bool) -> Option<String> {
        let mut st = self.st();
        let now = Self::ms(&st);
        let c = st
            .chans
            .iter_mut()
            .rev()
            .find(|c| c.b.client == client && c.b.handle == handle && c.freed_ms.is_none())?;
        c.freed_ms = Some(now);
        c.ok = Some(ok);
        let classes: Vec<String> = c.classes.iter().map(|k| format!("{k:#x}")).collect();
        let text = format!(
            "kf3: CHAN_LIFE free #N t_ms={now} key=({client:#x},{handle:#x}) host_token={:#x} lived_ms={} classes=[{}] ok={ok} vas={}",
            c.b.host_token,
            now.saturating_sub(c.born_ms),
            classes.join(","),
            pair(c.b.vas)
        );
        Self::chan_line(&mut st, text)
    }

    /// ★ The VA thread's look at the trigger file: at most once per [`TRIGGER_POLL_MS`]; fires
    /// once per new content (`Err` for content that does not parse).
    #[must_use]
    pub fn poll_trigger(&self, path: &Path, now: Instant) -> Option<Result<Trigger, String>> {
        {
            let mut st = self.st();
            if let Some(p) = st.trig_polled
                && now.saturating_duration_since(p).as_millis() < u128::from(TRIGGER_POLL_MS)
            {
                return None;
            }
            st.trig_polled = Some(now);
        }
        let f = std::fs::File::open(path).ok()?;
        let mut buf = Vec::with_capacity(TRIGGER_BYTES);
        f.take(TRIGGER_BYTES as u64).read_to_end(&mut buf).ok()?;
        let mut st = self.st();
        if st.trig_last.as_deref() == Some(buf.as_slice()) {
            return None;
        }
        st.trig_last = Some(buf.clone());
        drop(st);
        Some(
            parse_trigger(&buf)
                .ok_or_else(|| format!("trigger of {} byte(s) names no VA", buf.len())),
        )
    }

    /// ★★ **THE LOOKUP** — what was at `t.va` in the channel's space and in every tracked space
    /// whose history names it (at most [`PER_LOOKUP`] candidates), plus the primary space's last
    /// runs. Bounded lines; at most [`MANUAL_DUMPS`] per process (then ONE line says so).
    #[must_use]
    pub fn lookup(&self, t: &Trigger, view: &dyn SpaceView) -> Vec<String> {
        let x = t.va;
        let chid = t.chid.map_or("none".to_string(), |c| format!("{c:#x}"));
        // 1. Copy what is needed under the lock.
        let (n, t_ms, t0, header_counts, chans, mut cands) = {
            let mut st = self.st();
            if st.manual_dumps >= MANUAL_DUMPS {
                return vec![format!(
                    "kf3: VAHIST LOOKUP capped: {MANUAL_DUMPS} lookup(s) already answered; va={x:#x} chid={chid} not looked up"
                )];
            }
            st.manual_dumps = st.manual_dumps.saturating_add(1);
            let n = st.manual_dumps;
            let chans: Vec<ChanLife> = match t.chid {
                Some(c) => st
                    .chans
                    .iter()
                    .rev()
                    .filter(|l| l.b.host_token & 0xffff == c)
                    .take(PER_LOOKUP)
                    .cloned()
                    .collect(),
                None => Vec::new(),
            };
            let mut cands: Vec<Cand> = Vec::new();
            let add = |cands: &mut Vec<Cand>, st: &State, key: u64, role: &'static str| {
                if cands.len() >= PER_LOOKUP || cands.iter().any(|c| c.key == key) {
                    return;
                }
                if let Some(sp) = st.spaces.get(&key) {
                    cands.push(Cand {
                        key,
                        role,
                        space: Some(sp.clone()),
                        retired: None,
                    });
                } else if let Some(r) = st.retired.iter().rev().find(|r| r.key == key) {
                    cands.push(Cand {
                        key,
                        role,
                        space: Some(r.space.clone()),
                        retired: Some((
                            r.retired_ms,
                            r.extents
                                .iter()
                                .any(|&(s, l)| x.checked_sub(s).is_some_and(|d| d < l)),
                            r.extents_truncated,
                            r.live_at_retire,
                        )),
                    });
                } else {
                    cands.push(Cand {
                        key,
                        role,
                        space: None,
                        retired: None,
                    });
                }
            };
            for c in &chans {
                add(&mut cands, &st, c.b.vas, "chan");
            }
            let mut live: Vec<u64> = st
                .spaces
                .iter()
                .filter(|(_, s)| !s.covering(x, 1).is_empty())
                .map(|(&k, _)| k)
                .collect();
            live.sort_unstable();
            for k in live {
                add(&mut cands, &st, k, "live");
            }
            let retired: Vec<u64> = st
                .retired
                .iter()
                .rev()
                .filter(|r| {
                    r.extents
                        .iter()
                        .any(|&(s, l)| x.checked_sub(s).is_some_and(|d| d < l))
                        || !r.space.covering(x, 1).is_empty()
                })
                .map(|r| r.key)
                .collect();
            for k in retired {
                add(&mut cands, &st, k, "retired");
            }
            (
                n,
                Self::ms(&st),
                st.t0_unix_ms,
                (
                    st.spaces.len(),
                    st.retired.len(),
                    st.chans.len(),
                    st.spaces_untracked,
                ),
                chans,
                cands,
            )
        };
        // 2. The live rows (the lock is released: the view takes the rows' own locks).
        for k in view.live_keys() {
            if cands.len() >= PER_LOOKUP {
                break;
            }
            if !cands.iter().any(|c| c.key == k) && view.row_at(k, x).is_some() {
                cands.push(Cand {
                    key: k,
                    role: "live-row",
                    space: None,
                    retired: None,
                });
            }
        }
        // 3. Format.
        let (spaces, retired, nchans, untracked) = header_counts;
        let mut out = vec![cap(format!(
            "kf3: VAHIST LOOKUP #{n} va={x:#x} chid={chid} t_ms={t_ms} t0_unix_ms={t0} spaces={spaces} retired={retired} chans={nchans} chan_matches={} spaces_untracked={untracked}",
            chans.len()
        ))];
        for c in &chans {
            let classes: Vec<String> = c.classes.iter().map(|k| format!("{k:#x}")).collect();
            out.push(cap(format!(
                "kf3: VAHIST CHAN host_token={:#x} key=({:#x},{:#x}) guest_tok={:#x} pid={} engine={:#x} gp={} vas={} born_ms={} {} classes=[{}]",
                c.b.host_token,
                c.b.client,
                c.b.handle,
                c.b.guest_token,
                c.b.pid.map_or("none".to_string(), |p| format!("{p:#x}")),
                c.b.engine,
                c.b.gp_entries,
                pair(c.b.vas),
                c.born_ms,
                match (c.freed_ms, c.ok) {
                    (Some(f), ok) => format!("freed_ms={f} free_ok={}", ok.unwrap_or(false)),
                    (None, _) => "alive".to_string(),
                },
                classes.join(",")
            )));
        }
        if cands.is_empty() {
            out.push(cap(format!(
                "kf3: VAHIST VAS key=none role=none verdict=NOT_NAMED va={x:#x}: no tracked space, retired space or live row names it"
            )));
        }
        let mut primary: Option<(u64, Space)> = None;
        for c in &cands {
            let evs = c
                .space
                .as_ref()
                .map(|s| s.covering(x, PER_LOOKUP))
                .unwrap_or_default();
            let newest = evs.first().copied();
            let (verdict, row_text) = if let Some((_, cover, _, _)) = c.retired {
                (
                    if cover {
                        "RETIRED_MAPPED".to_string()
                    } else if let Some(e) = newest {
                        format!("RETIRED_{}", e.op.name())
                    } else {
                        "RETIRED_NOT_MAPPED".to_string()
                    },
                    String::new(),
                )
            } else if let Some((s, l, ram, off)) = view.row_at(c.key, x) {
                (
                    "PRESENT".to_string(),
                    format!(" row={s:#x}+{l:#x} ram={ram} off={off:#x}"),
                )
            } else {
                (
                    match newest.map(|e| e.op) {
                        None => "NOT_NAMED",
                        Some(Op::Unmap) => "UNMAPPED",
                        Some(Op::Refused) => "REFUSED",
                        Some(Op::Withheld) => "WITHHELD",
                        Some(Op::Held) => "HELD",
                        Some(Op::UnmapRefused) => "UNMAP_REFUSED",
                        Some(Op::Map) => "ROW_GONE",
                        Some(Op::Unknown | Op::WalkRefused) => "UNKNOWN",
                    }
                    .to_string(),
                    String::new(),
                )
            };
            let age = newest.map_or("none".to_string(), |e| {
                t_ms.saturating_sub(e.t_ms).to_string()
            });
            let (meta, complete) = match &c.space {
                Some(s) => (
                    format!(
                        " host_space={:#x} recycled={} born_ms={} runs={} walk_refusals={}{}",
                        s.host_space,
                        s.recycled,
                        s.born_ms,
                        s.runs,
                        s.walk_refusals,
                        s.last_refusal.map_or(String::new(), |e| format!(
                            " last_refusal=walk#{}/mask={:#x}/t_ms={}",
                            e.walk, e.aux, e.t_ms
                        ))
                    ),
                    if s.absent_evicted == 0 && s.sticky_evicted == 0 {
                        "yes".to_string()
                    } else {
                        format!("no(evicted {}+{})", s.absent_evicted, s.sticky_evicted)
                    },
                ),
                None => (" untracked".to_string(), "no(untracked)".to_string()),
            };
            let retired_text = c.retired.map_or(String::new(), |(ms, _, tr, live)| {
                format!(" retired_ms={ms} extents_truncated={tr} live_at_retire={live}")
            });
            out.push(cap(format!(
                "kf3: VAHIST VAS key={} role={} verdict={verdict} age_ms={age}{row_text} history_complete={complete}{meta}{retired_text}",
                pair(c.key),
                c.role
            )));
            for e in &evs {
                out.push(cap(format!(
                    "kf3: VAHIST EV key={} {}",
                    pair(c.key),
                    ev_text(e)
                )));
            }
            if c.retired.is_none()
                && let Some(p) = view.guest_pte(c.key, x)
            {
                out.push(cap(format!("kf3: VAHIST PTE key={} {p}", pair(c.key))));
            }
            if primary.is_none()
                && let Some(s) = &c.space
            {
                primary = Some((c.key, s.clone()));
            }
        }
        if let Some((key, s)) = primary {
            let len = s.last.len();
            for (i, e) in s.last.iter().enumerate() {
                out.push(cap(format!(
                    "kf3: VAHIST LAST key={} {}/{len} {}",
                    pair(key),
                    i.saturating_add(1),
                    ev_text(e)
                )));
            }
        }
        let lines = out.len().saturating_add(1);
        out.push(format!("kf3: VAHIST END #{n} lines={lines}"));
        out
    }
}

/// `KFWR_AP_*` of local video memory (`kf_walk.h`; the value `refusal_dump` uses).
const AP_VIDMEM: u8 = 0;

fn bit(raw: u64, b: u8) -> u64 {
    if b >= 64 { 0 } else { (raw >> b) & 1 }
}

fn vid_pde(fmt: &KfFormat, raw: u64) -> bool {
    let ap = aperture(fmt, raw);
    ap != fmt.pde_ap_invalid && fmt.pde_ap_map.get(usize::from(ap)).copied() == Some(AP_VIDMEM)
}

/// ★ **The guest's PTE for `va`, read from its own tables** (the walk's descriptor; every family
/// through [`KfFormat`]): the path from `pdb` to the dual level, the dual entry, and the big and
/// small PTEs it names. At most nine 8-byte reads; raw words are printed with a best-effort decode
/// (`refusal_dump::decode`), so the post-hoc reader can re-decode them. A table outside vidmem is
/// named, not read.
#[must_use]
pub fn probe_pte(
    fmt: &KfFormat,
    pdb: u64,
    va: u64,
    read8: &mut dyn FnMut(u64) -> Option<u64>,
) -> String {
    let dual = KF_DIRS - 1;
    let mut refused = false;
    let (steps, table) = {
        let mut rd = |a: u64| {
            let r = read8(a);
            if r.is_none() {
                refused = true;
            }
            r
        };
        path_to(fmt, pdb, va, dual, &mut rd)
    };
    let mut path = String::new();
    for s in &steps {
        let _ = write!(
            path,
            " L{}[{:#x}]@{:#x}={:#x}",
            s.level, s.idx, s.table, s.raw
        );
    }
    let Some(t) = table else {
        if refused {
            return format!(
                "va={va:#x} root={pdb:#x}: read refused after {} step(s):{path}",
                steps.len()
            );
        }
        return match steps.last() {
            Some(s) => format!(
                "va={va:#x} root={pdb:#x}: path ends at L{} ({}):{path}",
                s.level,
                decode(fmt, s.raw)
            ),
            None => format!("va={va:#x} root={pdb:#x}: path ends before the first level"),
        };
    };
    let Some(d) = fmt.dir.get(dual) else {
        return format!("va={va:#x}: no dual level in the descriptor");
    };
    let at = index_at(fmt, dual, va)
        .checked_mul(u64::from(d.entry_bytes))
        .and_then(|o| t.checked_add(o));
    let Some(at) = at else {
        return format!("va={va:#x}: dual entry address overflows");
    };
    let Some(lo) = read8(at) else {
        return format!("va={va:#x}: read refused at the dual entry @{at:#x}:{path}");
    };
    let hi = if d.entry_bytes >= 16 {
        read8(at.saturating_add(8))
    } else {
        Some(0)
    };
    let Some(hi) = hi else {
        return format!("va={va:#x}: read refused at the dual entry's high word @{at:#x}:{path}");
    };
    let mut s = format!("va={va:#x} root={pdb:#x}:{path} dual@{at:#x}=[lo={lo:#x} hi={hi:#x}]");
    if d.leaf_ps != KF_PS_NONE && bit(lo, fmt.valid_bit) == 1 {
        let _ = write!(s, " leaf_at_dual_level ({})", decode(fmt, lo));
        return s;
    }
    // Big half: the low word, its own address field; small half: the high word, a normal PDE.
    if vid_pde(fmt, lo) {
        let bt = field(lo, fmt.big_addr_local);
        let bi = (va >> fmt.big_va_lo) & u64::from(fmt.big_entries).wrapping_sub(1);
        let ba = bi
            .checked_mul(u64::from(fmt.big_entry_bytes))
            .and_then(|o| bt.checked_add(o));
        match ba.and_then(|a| read8(a).map(|r| (a, r))) {
            Some((a, r)) => {
                let _ = write!(
                    s,
                    " big@{a:#x}={r:#x} big_valid={} ({})",
                    bit(r, fmt.valid_bit),
                    decode(fmt, r)
                );
            }
            None => s.push_str(" big=read refused"),
        }
    } else {
        let _ = write!(s, " big_valid=none (ap {})", aperture(fmt, lo));
    }
    if d.entry_bytes >= 16 && vid_pde(fmt, hi) {
        let st = field(hi, fmt.addr_local);
        let si = (va >> fmt.small_va_lo) & u64::from(fmt.small_entries).wrapping_sub(1);
        let sa = si
            .checked_mul(u64::from(fmt.small_entry_bytes))
            .and_then(|o| st.checked_add(o));
        match sa.and_then(|a| read8(a).map(|r| (a, r))) {
            Some((a, r)) => {
                let _ = write!(
                    s,
                    " small@{a:#x}={r:#x} small_valid={} ({})",
                    bit(r, fmt.valid_bit),
                    decode(fmt, r)
                );
            }
            None => s.push_str(" small=read refused"),
        }
    } else {
        let _ = write!(s, " small_valid=none (ap {})", aperture(fmt, hi));
    }
    s
}
