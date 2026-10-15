//! Worker-only restricted USER ring relay. No vCPU/drainer caller may run a
//! turn: source-view preparation can call RM. Native USER privileges and own
//! mirrored mappings enforce access; scanning is never a completion authority.

use super::*;
use crate::mem::userread::{self, Mapping, Snapshot};
use kf_chan::user_loop::Stats;
use kf_chan::user_scan::{Fetch, Scan, Trigger};

/// Retry means no native publication/action was attempted. An uncertain write or
/// native doorbell error is always terminal; issued credits are never rolled back.
#[derive(Debug, PartialEq, Eq)]
enum Failure {
    Retry,
    Refused(String),
}
impl From<String> for Failure {
    fn from(v: String) -> Self {
        Self::Refused(v)
    }
}
impl From<&str> for Failure {
    fn from(v: &str) -> Self {
        Self::Refused(v.into())
    }
}
type Turn<T> = Result<T, Failure>;
fn locked<T>(v: std::sync::TryLockResult<T>, name: &'static str) -> Turn<T> {
    v.map_err(|e| match e {
        std::sync::TryLockError::WouldBlock => Failure::Retry,
        std::sync::TryLockError::Poisoned(_) => name.into(),
    })
}
fn source_error(e: userread::Refusal, phase: &'static str) -> Failure {
    match e {
        userread::Refusal::Busy | userread::Refusal::Changed => Failure::Retry,
        _ => format!("{phase}: {e:?}").into(),
    }
}
fn access_error(e: kf_rm::memory_list::AccessError, phase: &'static str) -> Failure {
    match e {
        kf_rm::memory_list::AccessError::Busy => Failure::Retry,
        _ => format!("{phase}: {e:?}").into(),
    }
}

/// One lifetime selected before any private native publication. Host numbering
/// is admitted only for the first ENG_SW child in this vertical slice.
#[derive(Clone)]
pub(super) struct Software {
    pub guest: u32,
    number: u16,
    pub live: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
}
impl Software {
    fn matches(&self, value: u32) -> bool {
        value == crate::defapi::NV50_DEFERRED_API_CLASS || value == u32::from(self.number)
    }
}
fn same_software(a: &Option<Software>, b: &Option<Software>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(&a.live, &b.live),
        _ => false,
    }
}
/// Classify and patch using ONE immutable channel-owned identity snapshot.
/// The later prefix scan uses the same snapshot; live admission is rechecked
/// before publication and before independently planned software actions.
fn stage_plan(scan: &Scan, software: &Option<Software>, words: &[u32]) -> Turn<(Vec<u32>, bool)> {
    let mut patched = Vec::new();
    patched
        .try_reserve_exact(words.len())
        .map_err(|_| "USER_RING_SCAN_ALLOC")?;
    patched.extend_from_slice(words);
    let mut scan = scan.clone();
    let mut at = 0usize;
    let mut rare = false;
    let sw = |v: u32| software.as_ref().is_some_and(|sw| sw.matches(v));
    while let Some(t) = scan
        .segment(words.get(at..).ok_or("USER_RING_SCAN_BOUND")?, sw)
        .map_err(|e| {
            // ★ 2026-10-14 diagnostic (owner Win11 runs 2980/2990: every reset follows a refusal
            // whose header looks like data — `Legacy`, `SubDeviceMask(0x107af0e0)`, `MethodOverflow`).
            // Name WHERE the scan stopped, so the next run tells a parser desync (a packet sized
            // differently from hardware) from a real legacy/mask packet. Bounded: one line per
            // refused channel (the relay stops at the refusal), at most 9 words of guest data.
            let detail = scan
                .locate(words.get(at..).unwrap_or(&[]), sw)
                .map(|l| scan_refusal_detail(&l, words, at))
                .unwrap_or_default();
            format!("USER_RING_SCAN {e:?}{detail}")
        })?
    {
        rare = true;
        let start = at.checked_add(t.start).ok_or("USER_RING_SCAN_BOUND")?;
        at = at.checked_add(t.end).ok_or("USER_RING_SCAN_BOUND")?;
        patched
            .get_mut(start..at)
            .ok_or("USER_RING_SCAN_BOUND")?
            .fill(0);
    }
    if rare && !scan.at_boundary() {
        return Err("USER_RING_TRIGGER_PARTIAL_SUFFIX_UNSUPPORTED".into());
    }
    Ok((patched, rare))
}
/// One bounded line: where in the GP segment (`words`, scanned from `base`) the scan refused, the
/// word there, the last header, the packet carried in from the previous segment and words
/// `[stop-4, stop+4]`. Diagnostic text only; guest words are data, never instructions.
fn scan_refusal_detail(l: &kf_chan::user_scan::Located, words: &[u32], base: usize) -> String {
    let stop = base.saturating_add(l.at);
    let lo = stop.saturating_sub(4);
    let hi = stop.saturating_add(5).min(words.len());
    let around: Vec<String> = words
        .get(lo..hi)
        .unwrap_or(&[])
        .iter()
        .map(|w| format!("{w:08x}"))
        .collect();
    let last = l
        .last_header
        .map(|(i, raw)| format!("{:#x}@{}", raw, base.saturating_add(i)))
        .unwrap_or_else(|| "none".into());
    let carried = l
        .carried
        .map(|(h, n)| {
            format!(
                "{:?} sub{} m{:#x} {}/{}",
                h.form, h.subchannel, h.method, n, h.arg_words
            )
        })
        .unwrap_or_else(|| "none".into());
    format!(
        " at word {stop}/{} raw={} last_header={last} carried_in={carried} words[{lo}..{hi}]=[{}]",
        words.len(),
        l.word
            .map_or_else(|| "end".into(), |w| format!("{w:#010x}")),
        around.join(" ")
    )
}

/// ★ 2026-10-14 MEASUREMENT ONLY — `KF3_USER_RING_TRACE=<dir>` (default off; never needed to boot,
/// never changes a decision). When a USER-ring scan refuses, the pump writes one bounded report per
/// refused channel into `<dir>`: the GP descriptor and its backing, the GP ring around the cursor,
/// the channel's last [`TRACE_HISTORY`] scanned segments (their words, capped), and three re-reads
/// of the SAME GP slot and segment 1/20/200 ms later, each rescanned from the same carried state —
/// which separates "the bytes changed under the scan" (a read-before-written race) from "the
/// grammar disagrees" (the same bytes refuse again). Guest words go only to that file.
/// Whether the measurement trace is on ([`trace_dir`]).
pub(super) fn tracing() -> bool {
    trace_dir().is_some()
}
fn trace_dir() -> Option<&'static std::path::Path> {
    static DIR: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        std::env::var_os("KF3_USER_RING_TRACE")
            .filter(|v| !v.is_empty() && v != "0")
            .map(std::path::PathBuf::from)
    })
    .as_deref()
}
const TRACE_HISTORY: usize = 64;
const TRACE_WORDS: usize = 16384;
/// Per-channel cap on the words the history retains (1 MiB).
const TRACE_HISTORY_WORDS: usize = 1 << 18;
/// Per-channel cap on straddle reports (a segment that ENDS inside a packet it started).
const TRACE_STRADDLES: u32 = 16;

struct SegTrace {
    seq: u64,
    at: std::time::Instant,
    cursor: u32,
    put: u32,
    entry: u64,
    woke: bool,
    carried_in: String,
    carried_out: String,
    rare: bool,
    backing: String,
    words: Vec<u32>,
    len: usize,
}

/// A deferred re-read of a straddling segment (no sleeping on the worker).
struct Reread {
    due: std::time::Instant,
    delay_ms: u64,
    /// `Some(scan time, carried-out text at scan)`: a VERIFY re-read of an ordinary VRAM-backed
    /// segment (counts stale reads); `None`: a straddle re-read.
    verify: Option<(std::time::Instant, String)>,
    seq: u64,
    cursor: u32,
    /// The scan-time snapshot's backing pieces and rows-log epoch.
    backing: (String, u64),
    /// Private-ring sequence this segment's descriptor got (issued count at scan time).
    seq_at_scan: u64,
    va: u64,
    bytes: usize,
    scan_in: Scan,
    words: Vec<u32>,
}

pub(super) struct Trace {
    born: std::time::Instant,
    seq: u64,
    history: std::collections::VecDeque<SegTrace>,
    history_words: usize,
    last_wake: Option<std::time::Instant>,
    woke: bool,
    reported: bool,
    straddles: u32,
    rereads: Vec<Reread>,
    /// Doorbell count read before the PUT read that found new work, and the one before it.
    db_at_put: u64,
    db_prev_put: u64,
    /// New-work PUT reads with / without a doorbell since the previous new-work PUT read.
    puts_with_db: u64,
    puts_without_db: u64,
    /// VERIFY re-reads done / found changed bytes / changed the scan's carried-out state or verdict.
    verified: u64,
    stale: u64,
    stale_desync: u64,
    stale_prefetch: u64,
    /// Stall detector: last progress marker, when it last changed, and whether reported.
    progress: ((u64, u32, u64, u64), std::time::Instant, bool),
    /// Idle detector: when the pump last had a turn, and whether an idle stall was reported.
    last_turn: std::time::Instant,
    idle_reported: bool,
    /// ★ 2026-10-14 (Minecraft Xid 32): what each private-ring descriptor was, by its issue
    /// sequence (the last [`TRACE_SLOTS`]), so a STALL can name the descriptor the PBDMA stopped at.
    slots: std::collections::VecDeque<(u64, String)>,
}
/// Private-ring descriptors remembered for a STALL dump (twice the private ring).
const TRACE_SLOTS: usize = 1024;
impl Trace {
    fn new() -> Self {
        Self {
            born: std::time::Instant::now(),
            seq: 0,
            history: std::collections::VecDeque::new(),
            history_words: 0,
            last_wake: None,
            woke: false,
            reported: false,
            straddles: 0,
            rereads: Vec::new(),
            db_at_put: 0,
            db_prev_put: 0,
            puts_with_db: 0,
            puts_without_db: 0,
            verified: 0,
            stale: 0,
            stale_desync: 0,
            stale_prefetch: 0,
            progress: ((0, 0, 0, 0), std::time::Instant::now(), false),
            last_turn: std::time::Instant::now(),
            idle_reported: false,
            slots: std::collections::VecDeque::new(),
        }
    }
    fn note_put(&mut self, doorbells: u64) {
        self.db_prev_put = self.db_at_put;
        self.db_at_put = doorbells;
        if doorbells > self.db_prev_put {
            self.puts_with_db += 1;
        } else {
            self.puts_without_db += 1;
        }
    }
    pub(super) fn wake(&mut self, woke: bool) {
        self.woke = woke;
        if woke {
            self.last_wake = Some(std::time::Instant::now());
        }
    }
}
fn carried_text(c: Option<(kf_abi::submit::MethodHeader, usize)>) -> String {
    c.map(|(h, n)| {
        format!(
            "{:?} sub{} m{:#x} {}/{}",
            h.form, h.subchannel, h.method, n, h.arg_words
        )
    })
    .unwrap_or_else(|| "none".into())
}
/// Diagnostic: the word positions the scanner reads as packet headers, fed one word at a time
/// from `scan` (stops at a refusal or END_PB_SEGMENT; at most `cap` headers).
fn header_chain(
    scan: &Scan,
    words: &[u32],
    sw: impl Fn(u32) -> bool,
    cap: usize,
) -> (Vec<(usize, u32)>, Option<usize>) {
    let mut s = scan.clone();
    let mut chain = Vec::new();
    for (i, w) in words.iter().enumerate() {
        if s.carried().is_none() {
            if chain.len() < cap {
                chain.push((i, *w));
            }
            if w >> 29 == 7 {
                return (chain, None);
            }
        }
        if s.segment(&[*w], &sw).is_err() {
            return (chain, Some(i));
        }
    }
    (chain, None)
}
/// Diagnostic: the packet the scan carries out of `words` when started from `scan` (fed one word
/// at a time, so software triggers do not stop it; END_PB_SEGMENT ends it). `Err(index)` on refusal.
fn scan_end(
    scan: &Scan,
    words: &[u32],
    sw: impl Fn(u32) -> bool,
) -> Result<Option<(kf_abi::submit::MethodHeader, usize)>, usize> {
    let mut s = scan.clone();
    for (i, w) in words.iter().enumerate() {
        if s.carried().is_none() && w >> 29 == 7 {
            return Ok(None);
        }
        s.segment(&[*w], &sw).map_err(|_| i)?;
    }
    Ok(s.carried())
}
fn words_text(out: &mut String, words: &[u32]) {
    use std::fmt::Write as _;
    for (i, chunk) in words.chunks(8).enumerate() {
        let _ = write!(out, "  {:5}:", i * 8);
        for w in chunk {
            let _ = write!(out, " {w:08x}");
        }
        out.push('\n');
    }
}

/// Native fetch credit and private staging retirement are separate. The first
/// representing descriptor carries the guest ring receipt, including a prefix
/// of zero bytes; no later suffix publication repeats it.
fn source_receipt(staged: bool, first: bool, generation: u64, get: u32) -> Option<(u64, u32)> {
    (!staged && first).then_some((generation, get))
}
/// ★ 2026-10-14 (Win11 Heaven D3D11 and Edge: host Xid 13 `ESR 0x404000=0x80000010` right after a USER
/// ring's growth migration) — the measured triggers of `FIFO_UPDATE_CHANNEL_INFO` sit INSIDE a front-end
/// atomic sequence (`NVC7C0_FE_ATOMIC_SEQUENCE_BEGIN` 0x148 … `_END` 0x14c on the compute subchannel,
/// `clc7c0.h`): `… 80002052 8001201e [2001a080 <handle>] 80002053`. Staging the prefix, waiting for its
/// witness, running the action and only then publishing the one-word suffix left the channel with an
/// OPEN atomic sequence and nothing to fetch for the whole action — long enough to be switched out —
/// where hardware services the software method with the PBDMA stalled on the method. The source
/// switch has no GPU-visible effect on the segment's remaining methods, so for this action alone the
/// whole remainder (the trigger zero-filled to `DMA_NOP`s by [`stage_plan`]) is published as ONE staged
/// copy: prefix, NOPs, suffix, contiguous. `Some((end, scan after the remainder))` when the remainder
/// after `trigger` holds no further trigger and ends at a packet boundary; `None` keeps the split.
fn gapless_remainder(
    scan_after_trigger: &Scan,
    words: &[u32],
    at: usize,
    trigger: Trigger,
    sw: impl Fn(u32) -> bool,
) -> Option<(usize, Scan)> {
    let after = at.checked_add(trigger.end)?;
    let mut tail = scan_after_trigger.clone();
    match tail.segment(words.get(after..)?, sw) {
        Ok(None) if tail.at_boundary() => Some((words.len(), tail)),
        _ => None,
    }
}
fn suffix_after_trigger(at: usize, trigger: Trigger, words: usize) -> Turn<(usize, bool)> {
    let next = at
        .checked_add(trigger.end)
        .filter(|n| *n <= words)
        .ok_or("USER_RING_SUFFIX_BOUND")?;
    Ok((next, next == words))
}

fn software_snapshot(me: &ChanPlane, r: &userrelay::Record) -> Turn<Option<Software>> {
    let pt = locked(me.pt.try_lock(), "USER_RING_PT_POISONED")?;
    let t = pt
        .get(&r.key)
        .filter(|t| t.user_ring && Arc::ptr_eq(&t.allocation, &r.allocation))
        .ok_or("USER_RING_ALLOCATION_GONE")?;
    let Some(sw) = &t.user_defapi else {
        return Ok(None);
    };
    if !sw.live.load(Ordering::Acquire) {
        return Err("USER_RING_SOFTWARE_OBJECT_GONE".into());
    }
    if !sw.ready.load(Ordering::Acquire) {
        return Err(Failure::Retry);
    }
    Ok(Some(sw.clone()))
}

pub(super) struct Pump {
    cache: crate::userstore::Cache,
    ram_generation: u64,
    generation: u64,
    source_admitted: bool,
    cursor: u32,
    scan: Scan,
    shadow: Option<super::usershadow::Shadow>,
    key: (u32, u32),
    fetch: Fetch,
    segment: Option<Segment>,
    guest_get: Option<(u64, u32)>,
    /// ★ 2026-10-14 (`kf_chan::user_loop`): what the last turn ended needing, and what this relay
    /// is waiting for (an event, a look, or the safety net -- never a tick).
    need: kf_chan::user_loop::Need,
    pub(super) wait: Option<kf_chan::user_loop::Wait>,
    /// Published guest work not yet followed by a batch marker.
    unnotified: bool,
    /// The most recent marker the relay published carries an interrupt (false = a control witness).
    marker_event: bool,
    /// Measurement only: when the outstanding SYNC_WAIT gate was authored.
    gate_at: Option<std::time::Instant>,
    /// ★ 2026-10-14 (measured: `GP_GET` runs ahead of processing): the reference-bearing markers
    /// published and observed PROCESSED (`USERD.REFERENCE`, tagged) -- the gate's proof.
    marks: kf_chan::user_gate::Marks,
    /// The marker count the outstanding SYNC_WAIT gate waits for.
    gate_target: u64,
    again: bool,
    /// ★ 2026-10-14 — `GP_ENTRY1_SYNC_WAIT` ([`kf_chan::user_gate::SyncGate`]): nothing at or
    /// after a guest SYNC_WAIT entry is READ before the hardware itself would fetch it.
    sync: kf_chan::user_gate::SyncGate,
    /// ★ 2026-10-14: the bounded retry of a scan refusal ([`kf_chan::user_gate::Grace`]), and how
    /// many retries this channel has reported (the first few only).
    grace: kf_chan::user_gate::Grace,
    grace_reported: u32,
    grace_was_retrying: bool,
    /// ★ 2026-10-14: guest SYNC_WAIT entries gated while a packet was open (control witness).
    sync_carried: u64,
    /// Guest GP entries relayed by the turn in progress (`Stats::record_turn`).
    seg_turn: u32,
    /// ★ 2026-10-14 batching (doc §9): the host doorbell this claim owes for what it published. Rung
    /// ONCE at the end of the chain (`Pump::ring_owed`), never per publication.
    owed: bool,
    /// The `GP_PUT` this claim read (with the source generation it applies to): later iterations of
    /// the chain reuse it. The token's flag was cleared before the read, so a doorbell that lands
    /// after it makes the claim run again; a stale value only means fewer entries now.
    chain_put: Option<(u64, u32)>,
    /// This iteration is not the first of the claim: the `GP_GET`/`REFERENCE` observation and the
    /// guest `GP_GET` write-back were done by the first.
    chained: bool,
    /// The iteration just run needs a turn of its own (a staged segment, a shadow observation).
    split: bool,
    /// Measurement only ([`trace_dir`]).
    pub(super) trace: Option<Box<Trace>>,
}

struct Segment {
    ring: Option<Mapping<'static>>,
    software: Option<Software>,
    source: Snapshot<'static>,
    pb: Snapshot<'static>,
    entry: u64,
    words: Vec<u32>,
    patched: Vec<u32>,
    at: usize,
    generation: u64,
    guest_get: u32,
    rare: bool,
    staged: bool,
    /// Counted once in `Stats` (a staged segment is published in several steps).
    rare_counted: bool,
    wait: Option<Wait>,
}
struct Wait {
    witness: u64,
    trigger: Option<Trigger>,
    action: Option<PendingAction>,
    /// ★ The trigger's suffix was published with its prefix ([`gapless_remainder`]).
    gapless: bool,
}
enum PendingAction {
    Invalidate {
        ticket: u64,
        plan: crate::defapi::Planned,
    },
    Context {
        cell: SwCell,
        plan: crate::defapi::Planned,
    },
}

impl Pump {
    pub(super) fn new(me: &ChanPlane, subdevice_id: u32, key: (u32, u32)) -> Result<Self, String> {
        Ok(Self {
            cache: crate::userstore::Cache::new(
                me.rm,
                me.store,
                store_read_bound(me.layout.carve()),
            )
            .map_err(|e| format!("USER_RING_CACHE {e:?}"))?,
            ram_generation: me.ram.try_generation().ok_or("USER_RING_RAM_TOPOLOGY")?,
            generation: 1,
            source_admitted: false,
            cursor: 0,
            // The native birth authors this same admitted channel identity.
            scan: if userrelay::shadow_enabled() {
                Scan::default()
            } else {
                Scan::for_subdevice(subdevice_id).map_err(str::to_owned)?
            },
            shadow: userrelay::shadow_enabled()
                .then(|| super::usershadow::Shadow::new(subdevice_id)),
            key,
            fetch: Fetch::new(kf_chan::user_storage::ENTRIES).map_err(str::to_owned)?,
            segment: None,
            guest_get: None,
            need: kf_chan::user_loop::Need::Idle,
            wait: None,
            unnotified: false,
            marker_event: false,
            gate_at: None,
            seg_turn: 0,
            owed: false,
            chain_put: None,
            chained: false,
            split: false,
            marks: kf_chan::user_gate::Marks::default(),
            gate_target: 0,
            again: false,
            sync: kf_chan::user_gate::SyncGate::default(),
            grace: kf_chan::user_gate::Grace::default(),
            grace_reported: 0,
            grace_was_retrying: false,
            sync_carried: 0,
            trace: trace_dir().map(|_| Box::new(Trace::new())),
        })
    }
    pub(super) fn release(&mut self) -> Result<(), String> {
        if self.sync_carried != 0 {
            eprintln!(
                "kf3: USER_RING_SYNC_CARRIED SUMMARY {:#x}:{:#x} sync_wait_entries_behind_an_open_packet_read_ungated={}",
                self.key.0, self.key.1, self.sync_carried
            );
        }
        if let Some(t) = &self.trace {
            eprintln!(
                "kf3: USER_RING_TRACE SUMMARY {:x?} life_ms={} segments={} puts_with_doorbell={} puts_without_doorbell={} straddles={} verified={} stale={} stale_desync={} stale_prefetch={}",
                self.key,
                t.born.elapsed().as_millis(),
                t.seq,
                t.puts_with_db,
                t.puts_without_db,
                t.straddles,
                t.verified,
                t.stale,
                t.stale_desync,
                t.stale_prefetch
            );
        }
        if let Some(shadow) = self.shadow.take() {
            eprintln!(
                "kf3: USER_RING_SHADOW_SUMMARY {:x?} observation_attempt_counts={:?}; never execution or completion",
                self.key, shadow.counts
            );
        }
        self.cache
            .release()
            .map_err(|e| format!("USER_RING_SOURCE_CLEANUP {e:?}"))
    }

    fn snapshot(
        &mut self,
        me: &ChanPlane,
        r: &userrelay::Record,
        va: u64,
        bytes: usize,
    ) -> Turn<Snapshot<'static>> {
        userread::snapshot(
            &r.mirror.rows,
            &r.mirror.log,
            me.ram,
            self.ram_generation,
            va,
            bytes,
            &mut self.cache,
        )
        .map_err(|e| source_error(e, "USER_RING_SOURCE"))
    }

    /// Measurement only: remember this scanned segment (bounded history, [`trace_dir`]).
    #[allow(clippy::too_many_arguments)]
    fn trace_segment(
        &mut self,
        r: &userrelay::Record,
        put: u32,
        entry: u64,
        d: kf_abi::submit::GpfifoEntry,
        pb: &Snapshot<'static>,
        words: &[u32],
        rare: bool,
        software: &Option<Software>,
    ) {
        let sw = |v: u32| software.as_ref().is_some_and(|s| s.matches(v));
        let start = self.scan.carried();
        let end = scan_end(&self.scan, words, sw);
        let carried_in = carried_text(start);
        let carried_out = match end {
            Ok(c) => carried_text(c),
            Err(i) => format!("refused@{i}"),
        };
        let cursor = self.cursor;
        let issued_now = self.fetch.issued();
        let scan_in = self.scan.clone();
        let Some(t) = self.trace.as_mut() else {
            return;
        };
        t.seq += 1;
        let kept = words[..words.len().min(TRACE_WORDS)].to_vec();
        // A segment that starts at a packet boundary and ENDS inside a packet it started: legal
        // hardware framing, but the shape a desync leaves behind. Report it with deferred re-reads.
        if start.is_none() && matches!(end, Ok(Some(_))) && t.straddles < TRACE_STRADDLES {
            t.straddles += 1;
            let now = std::time::Instant::now();
            for delay_ms in [1u64, 20, 200] {
                t.rereads.push(Reread {
                    due: now + std::time::Duration::from_millis(delay_ms),
                    delay_ms,
                    verify: None,
                    seq: t.seq,
                    cursor,
                    backing: (pb.describe(), pb.epoch),
                    seq_at_scan: issued_now,
                    va: d.gpu_va,
                    bytes: d.len_bytes as usize,
                    scan_in: scan_in.clone(),
                    words: kept.clone(),
                });
            }
            let (chain, _) = header_chain(&scan_in, words, sw, 64);
            let tail: Vec<String> = chain
                .iter()
                .rev()
                .take(6)
                .rev()
                .map(|(i, w)| format!("{i}:{w:08x}"))
                .collect();
            eprintln!(
                "kf3: USER_RING_TRACE STRADDLE {:#x}:{:#x} seq={} cursor={cursor} put={put} entry={entry:#018x} pb_va={:#x} words={} carried_out={carried_out} last_headers=[{}] backing=[{}] db_at_put={} db_prev_put={}",
                r.key.0,
                r.key.1,
                t.seq,
                d.gpu_va,
                words.len(),
                tail.join(" "),
                pb.describe(),
                t.db_at_put,
                t.db_prev_put
            );
        }
        // VERIFY: every VRAM-backed segment of at most 1024 words (the first 20000 per channel)
        // is read again 0.2 and 2 ms after its scan: how often, and for how long, does the scan
        // read bytes that are not the final ones?
        if pb.describe().contains("vram")
            && words.len() <= 1024
            && t.verified + (t.rereads.len() as u64) < 40_000
        {
            let now = std::time::Instant::now();
            for delay_us in [200u64, 2000] {
                t.rereads.push(Reread {
                    due: now + std::time::Duration::from_micros(delay_us),
                    delay_ms: delay_us,
                    verify: Some((now, carried_out.clone())),
                    seq: t.seq,
                    cursor,
                    backing: (pb.describe(), pb.epoch),
                    seq_at_scan: issued_now,
                    va: d.gpu_va,
                    bytes: d.len_bytes as usize,
                    scan_in: scan_in.clone(),
                    words: kept.clone(),
                });
            }
        }
        while t.history.len() == TRACE_HISTORY
            || (t.history_words + kept.len() > TRACE_HISTORY_WORDS && !t.history.is_empty())
        {
            if let Some(old) = t.history.pop_front() {
                t.history_words -= old.words.len();
            }
        }
        t.history_words += kept.len();
        t.history.push_back(SegTrace {
            seq: t.seq,
            at: std::time::Instant::now(),
            cursor,
            put,
            entry,
            woke: t.woke,
            carried_in,
            carried_out,
            rare,
            backing: pb.describe(),
            words: kept,
            len: words.len(),
        });
    }

    /// Measurement only: name the descriptors the last publication issued (the last `what.len()`
    /// sequence numbers). `what` runs only with the trace on.
    fn trace_slots(&mut self, what: impl FnOnce() -> Vec<String>) {
        let issued = self.fetch.issued();
        let Some(t) = self.trace.as_mut() else {
            return;
        };
        let what = what();
        let first = issued.saturating_sub(what.len() as u64);
        for (i, d) in what.into_iter().enumerate() {
            if t.slots.len() == TRACE_SLOTS {
                t.slots.pop_front();
            }
            t.slots.push_back((first + i as u64, d));
        }
    }

    /// Measurement only: report once when this pump has made no progress (cursor, ring
    /// generation, private-ring issue/fetch counts) for 1.5 s while turns keep arriving.
    fn trace_stall(&mut self, me: &ChanPlane, r: &userrelay::Record) {
        let marker = (
            self.generation,
            self.cursor,
            self.fetch.issued(),
            self.fetch.fetched(),
        );
        let now = std::time::Instant::now();
        let Some(t) = self.trace.as_mut() else {
            return;
        };
        if t.progress.0 != marker {
            t.progress = (marker, now, false);
            return;
        }
        if t.progress.2 || now.duration_since(t.progress.1) < std::time::Duration::from_millis(1500)
        {
            return;
        }
        t.progress.2 = true;
        let mut b = [0u8; 4];
        let put = r
            .userd
            .read(
                me,
                r.key,
                &r.allocation,
                kf_abi::submit::USERD_GP_PUT,
                &mut b,
            )
            .ok()
            .map(|()| u32::from_le_bytes(b));
        let native_get = r.lease.storage.lock().ok().and_then(|s| s.gpu_get().ok());
        let segment = self.segment.as_ref().map(|s| {
            format!(
                "at={}/{} staged={} rare={} wait_witness={:?} wait_trigger={}",
                s.at,
                s.words.len(),
                s.staged,
                s.rare,
                s.wait.as_ref().map(|w| w.witness),
                s.wait.as_ref().is_some_and(|w| w.trigger.is_some())
            )
        });
        eprintln!(
            "kf3: USER_RING_TRACE STALL {:#x}:{:#x} no progress 1.5 s: generation={} cursor={} guest_put={put:?} fetch_issued={} fetch_fetched={} native_get={native_get:?} sync={:?} grace_retrying={} segment={segment:?} guest_get_pending={:?}",
            r.key.0,
            r.key.1,
            self.generation,
            self.cursor,
            self.fetch.issued(),
            self.fetch.fetched(),
            self.sync,
            self.grace.retrying(),
            self.guest_get
        );
        // ★ 2026-10-14: where the PBDMA stopped — our private USERD as last written back, the
        // descriptors around the private GP_GET, and the scanned segment holding the PB GET.
        let userd = r
            .lease
            .storage
            .lock()
            .ok()
            .and_then(|s| s.userd_words().ok());
        let fetched = self.fetch.fetched();
        if let Some(u) = userd {
            let pb_get = u64::from(u[17]) | (u64::from(u[24]) << 32);
            let top = u64::from(u[22]) | (u64::from(u[23]) << 32);
            eprintln!(
                "kf3: USER_RING_TRACE STALL_USERD {:#x}:{:#x} pb_get={pb_get:#x} top_level_get={top:#x} pb_put={:#x} gp_get={} gp_put={} ref={:#x} fetched_seq={fetched}",
                r.key.0,
                r.key.1,
                u64::from(u[16]) | (u64::from(u[19]) << 32),
                u[34],
                u[35],
                u[18]
            );
            let Some(t) = self.trace.as_ref() else {
                return;
            };
            for (seq, d) in t
                .slots
                .iter()
                .filter(|(q, _)| q + 8 >= fetched && *q <= fetched + 8)
            {
                let mark = if *seq == fetched {
                    " <== private GP_GET"
                } else {
                    ""
                };
                eprintln!(
                    "kf3: USER_RING_TRACE STALL_SLOT {:#x}:{:#x} seq={seq} {d}{mark}",
                    r.key.0, r.key.1
                );
            }
            // The scanned guest segment the PB GET lies in (its words at scan time).
            let hit = t.history.iter().rev().find_map(|h| {
                let d = kf_abi::submit::gp_entry_decode_direct_c56f(h.entry)?;
                (pb_get >= d.gpu_va && pb_get <= d.gpu_va + d.len_bytes)
                    .then(|| (h, d.gpu_va, ((pb_get - d.gpu_va) / 4) as usize))
            });
            match hit {
                Some((h, va, at)) => {
                    let lo = at.saturating_sub(8);
                    let hi = (at + 8).min(h.words.len());
                    let words: Vec<String> = h
                        .words
                        .get(lo..hi)
                        .unwrap_or(&[])
                        .iter()
                        .map(|w| format!("{w:08x}"))
                        .collect();
                    eprintln!(
                        "kf3: USER_RING_TRACE STALL_SEGMENT {:#x}:{:#x} seq={} cursor={} entry={:#018x} pb_va={va:#x} words={} carried_in={} carried_out={} rare={} pb_get_word={at} scan_words[{lo}..{hi}]=[{}]",
                        r.key.0,
                        r.key.1,
                        h.seq,
                        h.cursor,
                        h.entry,
                        h.len,
                        h.carried_in,
                        h.carried_out,
                        h.rare,
                        words.join(" ")
                    );
                }
                None => eprintln!(
                    "kf3: USER_RING_TRACE STALL_SEGMENT {:#x}:{:#x} pb_get={pb_get:#x} in no remembered guest segment (private staging/witness, or older than the history)",
                    r.key.0, r.key.1
                ),
            }
            // ★ 2026-10-14 (Heaven D3D11 Xid 13): the last segments the relay scanned, words as
            // scanned (head and tail, bounded: 8 segments x 56 words, once per stall).
            for h in t
                .history
                .iter()
                .rev()
                .take(8)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
            {
                let head: Vec<String> = h
                    .words
                    .iter()
                    .take(48)
                    .map(|w| format!("{w:08x}"))
                    .collect();
                let tail: Vec<String> = h
                    .words
                    .iter()
                    .skip(h.words.len().saturating_sub(8).max(48))
                    .map(|w| format!("{w:08x}"))
                    .collect();
                eprintln!(
                    "kf3: USER_RING_TRACE STALL_HISTORY {:#x}:{:#x} seq={} cursor={} entry={:#018x} words={} carried_in={} carried_out={} rare={} head=[{}] tail=[{}]",
                    r.key.0,
                    r.key.1,
                    h.seq,
                    h.cursor,
                    h.entry,
                    h.len,
                    h.carried_in,
                    h.carried_out,
                    h.rare,
                    head.join(" "),
                    tail.join(" ")
                );
            }
        }
    }

    /// Measurement only: run the straddle re-reads that are due (never sleeps).
    fn trace_due(&mut self, me: &ChanPlane, r: &userrelay::Record, software: &Option<Software>) {
        let now = std::time::Instant::now();
        let due: Vec<Reread> = match self.trace.as_mut() {
            Some(t) if t.rereads.iter().any(|x| x.due <= now) => {
                let (due, later) = std::mem::take(&mut t.rereads)
                    .into_iter()
                    .partition(|x| x.due <= now);
                t.rereads = later;
                due
            }
            _ => return,
        };
        let sw = |v: u32| software.as_ref().is_some_and(|s| s.matches(v));
        for x in due {
            let mut backing_now = None;
            let fresh: Option<Vec<u32>> = self.snapshot(me, r, x.va, x.bytes).ok().map(|s| {
                backing_now = Some((s.describe(), s.epoch));
                s.bytes
                    .chunks_exact(4)
                    .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
                    .collect()
            });
            let moved = backing_now.as_ref().map(|b| b.0 != x.backing.0);
            let epoch = backing_now.as_ref().map(|b| (x.backing.1, b.1));
            let Some(fresh) = fresh else {
                if x.verify.is_some() {
                    continue;
                }
                eprintln!(
                    "kf3: USER_RING_TRACE STRADDLE_REREAD {:#x}:{:#x} seq={} +{}ms unreadable",
                    r.key.0, r.key.1, x.seq, x.delay_ms
                );
                continue;
            };
            let diffs: Vec<usize> = (0..fresh.len().min(x.words.len()))
                .filter(|&i| fresh[i] != x.words[i])
                .collect();
            let end = match scan_end(&x.scan_in, &fresh, sw) {
                Ok(c) => carried_text(c),
                Err(i) => format!("refused@{i}"),
            };
            if let Some((scanned_at, end_at_scan)) = &x.verify {
                let Some(t) = self.trace.as_mut() else {
                    continue;
                };
                t.verified += 1;
                let mut write_file = false;
                if !diffs.is_empty() {
                    // Before the GPU began this entry: the GPU will execute the NEW bytes, the
                    // scan saw the old ones — a real mismatch. After: possibly a legitimate reuse.
                    let prefetch = self.fetch.fetched() <= x.seq_at_scan;
                    if prefetch {
                        t.stale_prefetch += 1;
                    }
                    t.stale += 1;
                    let desync = *end_at_scan != end;
                    if desync {
                        t.stale_desync += 1;
                    }
                    write_file = t.stale <= 8;
                    if t.stale <= 64 || desync && t.stale_desync <= 64 {
                        eprintln!(
                            "kf3: USER_RING_TRACE STALE {:#x}:{:#x} seq={} after_us={} words={} changed={} first_diff={:?} last_diff={:?} carried_out_at_scan={end_at_scan} now={end} backing_moved={moved:?} rows_epoch(scan,now)={epoch:?} backing_at_scan=[{}] prefetch={} verified={} stale={} stale_desync={} stale_prefetch={}",
                            r.key.0,
                            r.key.1,
                            x.seq,
                            now.duration_since(*scanned_at).as_micros(),
                            x.words.len(),
                            diffs.len(),
                            diffs.first(),
                            diffs.last(),
                            x.backing.0,
                            prefetch,
                            t.verified,
                            t.stale,
                            t.stale_desync,
                            t.stale_prefetch
                        );
                    }
                }
                if t.verified % 5000 == 0 {
                    eprintln!(
                        "kf3: USER_RING_TRACE VERIFY {:#x}:{:#x} verified={} stale={} stale_desync={} stale_prefetch={} doorbell_census={:x?}",
                        r.key.0,
                        r.key.1,
                        t.verified,
                        t.stale,
                        t.stale_desync,
                        t.stale_prefetch,
                        me.doorbell_census_now()
                    );
                }
                if write_file && let Some(dir) = trace_dir() {
                    let mut out = format!(
                        "STALE {:#x}:{:#x} seq={} entry_cursor={} pump_cursor_now={} after_us={} pb_va={:#x} bytes={} fetch_issued={} fetch_fetched={}\nas scanned:\n",
                        r.key.0,
                        r.key.1,
                        x.seq,
                        x.cursor,
                        self.cursor,
                        now.duration_since(*scanned_at).as_micros(),
                        x.va,
                        x.bytes,
                        self.fetch.issued(),
                        self.fetch.fetched()
                    );
                    words_text(&mut out, &x.words);
                    out.push_str("re-read:\n");
                    words_text(&mut out, &fresh);
                    out.push_str("gp ring now (index: entry):\n");
                    let entries = r.source.1;
                    if entries.is_power_of_two() && entries >= 2 {
                        for k in 0..24u32 {
                            let idx = x
                                .cursor
                                .wrapping_add(entries)
                                .wrapping_sub(4)
                                .wrapping_add(k)
                                & (entries - 1);
                            let at = r.source.0 + u64::from(idx) * 8;
                            let e = self.snapshot(me, r, at, 8).ok().and_then(|s| {
                                s.bytes.as_slice().try_into().ok().map(u64::from_le_bytes)
                            });
                            out.push_str(&format!("  {idx}: {e:#018x?}\n"));
                        }
                    }
                    let _ = std::fs::create_dir_all(dir).and_then(|()| {
                        std::fs::write(
                            dir.join(format!(
                                "user-ring-stale-{:x}-{:x}-{}-{}.txt",
                                r.key.0, r.key.1, x.seq, x.delay_ms
                            )),
                            out.as_bytes(),
                        )
                    });
                }
                continue;
            }
            eprintln!(
                "kf3: USER_RING_TRACE STRADDLE_REREAD {:#x}:{:#x} seq={} +{}ms changed={} first_diff={:?} carried_out_now={end} backing_moved={moved:?} rows_epoch(scan,now)={epoch:?}",
                r.key.0,
                r.key.1,
                x.seq,
                x.delay_ms,
                diffs.len(),
                diffs.first()
            );
            if x.delay_ms == 200 {
                if let Some(dir) = trace_dir() {
                    let mut out = format!(
                        "STRADDLE {:#x}:{:#x} seq={} pb_va={:#x} bytes={}\nas scanned:\n",
                        r.key.0, r.key.1, x.seq, x.va, x.bytes
                    );
                    words_text(&mut out, &x.words);
                    out.push_str("re-read +200ms:\n");
                    words_text(&mut out, &fresh[..fresh.len().min(TRACE_WORDS)]);
                    let _ = std::fs::create_dir_all(dir).and_then(|()| {
                        std::fs::write(
                            dir.join(format!(
                                "user-ring-straddle-{:x}-{:x}-{}.txt",
                                r.key.0, r.key.1, x.seq
                            )),
                            out.as_bytes(),
                        )
                    });
                }
            }
        }
    }

    /// Measurement only ([`trace_dir`]): one bounded report for a refused segment. Runs on the
    /// worker (never a vCPU or the drainer), once per channel; it sleeps 221 ms in total for the
    /// three re-reads, which is acceptable only because the flag is a measurement and the channel
    /// is about to be declared dead anyway. Decides nothing.
    #[allow(clippy::too_many_arguments)]
    fn trace_refusal(
        &mut self,
        me: &ChanPlane,
        r: &userrelay::Record,
        (put, gp_va, entry): (u32, u64, u64),
        d: kf_abi::submit::GpfifoEntry,
        pb: &Snapshot<'static>,
        words: &[u32],
        software: &Option<Software>,
        why: &Failure,
    ) {
        use std::fmt::Write as _;
        let Some(dir) = trace_dir() else {
            return;
        };
        if self.trace.as_ref().is_none_or(|t| t.reported) {
            return;
        }
        let sw = |v: u32| software.as_ref().is_some_and(|s| s.matches(v));
        let entries = r.source.1;
        let now = std::time::Instant::now();
        let (born, woke, since_wake) = self
            .trace
            .as_ref()
            .map(|t| {
                (
                    now.duration_since(t.born).as_millis(),
                    t.woke,
                    t.last_wake.map(|w| now.duration_since(w).as_micros()),
                )
            })
            .unwrap_or_default();
        let (db_at, db_prev, pw, pwo) = self
            .trace
            .as_ref()
            .map(|t| {
                (
                    t.db_at_put,
                    t.db_prev_put,
                    t.puts_with_db,
                    t.puts_without_db,
                )
            })
            .unwrap_or_default();
        let mut out = String::new();
        let gp_backing = self
            .snapshot(me, r, gp_va, 8)
            .map(|s| s.describe())
            .unwrap_or_else(|e| format!("{e:?}"));
        let _ = writeln!(
            out,
            "USER_RING_TRACE refusal key={:#x}:{:#x} idx={} cursor={} put={put} entries={entries} gp_ring={:#x} gp_va={gp_va:#x} entry={entry:#018x} pb_va={:#x} pb_bytes={} sync_wait={} subroutine={} generation={} fetch_issued={} fetch_fetched={} pump_age_ms={born} woke_by_request={woke} since_last_request_us={since_wake:?} doorbells_at_put_read={db_at} at_previous_new_put={db_prev} now={} puts_with_doorbell={pw} puts_without_doorbell={pwo}\n  pb_backing=[{}] gp_backing=[{gp_backing}]\nwhy: {why:?}\ncarried_in: {}",
            r.key.0,
            r.key.1,
            r.idx,
            self.cursor,
            r.source.0,
            d.gpu_va,
            d.len_bytes,
            d.sync_wait,
            d.subroutine,
            self.generation,
            self.fetch.issued(),
            self.fetch.fetched(),
            me.queued(r.idx),
            pb.describe(),
            carried_text(self.scan.carried()),
        );
        let (chain, stop) = header_chain(&self.scan, words, sw, 512);
        let _ = writeln!(out, "header chain (index:raw) stop={stop:?}:");
        for c in chain.chunks(8) {
            let line: Vec<String> = c.iter().map(|(i, w)| format!("{i}:{w:08x}")).collect();
            let _ = writeln!(out, "  {}", line.join(" "));
        }
        // The GP ring around the cursor, as it reads NOW.
        let _ = writeln!(out, "gp ring now (index: entry):");
        if entries.is_power_of_two() && entries >= 2 {
            for k in 0..24u32 {
                let idx = self
                    .cursor
                    .wrapping_add(entries)
                    .wrapping_sub(16)
                    .wrapping_add(k)
                    & (entries - 1);
                let at = r.source.0 + u64::from(idx) * 8;
                let e = self
                    .snapshot(me, r, at, 8)
                    .ok()
                    .and_then(|s| s.bytes.as_slice().try_into().ok().map(u64::from_le_bytes));
                let mark = if idx == self.cursor { " <= cursor" } else { "" };
                let _ = writeln!(out, "  {idx}: {e:#018x?}{mark}");
            }
        }
        // Re-read the same GP slot, the same segment and USERD PUT later; rescan from the same state.
        let mut summary = Vec::new();
        for delay in [1u64, 20, 200] {
            std::thread::sleep(std::time::Duration::from_millis(delay));
            let entry2 = self
                .snapshot(me, r, gp_va, 8)
                .ok()
                .and_then(|s| s.bytes.as_slice().try_into().ok().map(u64::from_le_bytes));
            let mut put2 = [0u8; 4];
            let put2 = r
                .userd
                .read(
                    me,
                    r.key,
                    &r.allocation,
                    kf_abi::submit::USERD_GP_PUT,
                    &mut put2,
                )
                .ok()
                .map(|()| u32::from_le_bytes(put2));
            let mut backing2 = None;
            let words2: Option<Vec<u32>> = self
                .snapshot(me, r, d.gpu_va, d.len_bytes as usize)
                .ok()
                .map(|s| {
                    backing2 = Some(format!(
                        "[{}] epoch {}->{}",
                        s.describe(),
                        pb.epoch,
                        s.epoch
                    ));
                    s.bytes
                        .chunks_exact(4)
                        .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
                        .collect()
                });
            let (changed, first) = words2.as_ref().map_or((None, None), |w2| {
                let diffs: Vec<usize> = (0..w2.len().min(words.len()))
                    .filter(|&i| w2[i] != words[i])
                    .collect();
                (Some(diffs.len()), diffs.first().copied())
            });
            let rescan = words2.as_ref().map(|w2| {
                self.scan.locate(w2, sw).map_or_else(
                    || "clean".to_owned(),
                    |l| format!("{:?}@{}", l.refusal, l.at),
                )
            });
            let line = format!(
                "reread+{delay}ms entry_same={:?} put={put2:?} doorbells={} words_changed={changed:?} first_diff={first:?} rescan={rescan:?} backing={backing2:?}",
                entry2.map(|e| e == entry),
                me.queued(r.idx)
            );
            let _ = writeln!(out, "{line}");
            if let (Some(w2), Some(n)) = (&words2, changed) {
                if n > 0 {
                    let _ = writeln!(out, "  reread words:");
                    words_text(&mut out, &w2[..w2.len().min(TRACE_WORDS)]);
                }
            }
            summary.push(format!(
                "+{delay}ms:db={}:changed={}:rescan={}",
                me.queued(r.idx),
                changed.map_or("?".into(), |n| n.to_string()),
                rescan.unwrap_or_else(|| "?".into())
            ));
        }
        let _ = writeln!(out, "failing segment words ({}):", words.len());
        words_text(&mut out, &words[..words.len().min(TRACE_WORDS)]);
        if let Some(t) = self.trace.as_mut() {
            let _ = writeln!(out, "history (oldest first, {} segments):", t.history.len());
            for s in &t.history {
                let _ = writeln!(
                    out,
                    "-- seq={} age_ms={} cursor={} put={} entry={:#018x} woke_by_request={} rare={} carried_in={} carried_out={} backing=[{}] words={}",
                    s.seq,
                    now.duration_since(s.at).as_millis(),
                    s.cursor,
                    s.put,
                    s.entry,
                    s.woke,
                    s.rare,
                    s.carried_in,
                    s.carried_out,
                    s.backing,
                    s.len
                );
                words_text(&mut out, &s.words);
            }
            t.reported = true;
        }
        let name = format!(
            "user-ring-refusal-{:x}-{:x}-{}.txt",
            r.key.0, r.key.1, self.cursor
        );
        let written = std::fs::create_dir_all(dir)
            .and_then(|()| std::fs::write(dir.join(&name), out.as_bytes()));
        eprintln!(
            "kf3: USER_RING_TRACE {:#x}:{:#x} cursor={} put={put} entry={entry:#018x} pb_va={:#x} bytes={} doorbell_census(translated,passthrough,other,last_other)={:x?} db_at_put={db_at} db_prev_put={db_prev} puts_with_db={pw} puts_without_db={pwo} {} file={name} written={}",
            r.key.0,
            r.key.1,
            self.cursor,
            d.gpu_va,
            d.len_bytes,
            me.doorbell_census_now(),
            summary.join(" "),
            written.is_ok()
        );
    }

    /// ★ 2026-10-14 (`docs/design/V3_USER_RING_EVENT_LOOP_20261014.md` §3.2): a turn ends in a
    /// [`Need`](kf_chan::user_loop::Need) -- progress (`Again`), nothing (`Idle`), or what it waits
    /// for. No tick: the caller arms the wake ([`ChanPlane::user_ring_serve`]).
    fn turn(&mut self, me: &ChanPlane, r: &mut userrelay::Record) -> Turn<bool> {
        use kf_chan::user_loop::Need;
        self.need = Need::Idle;
        self.again = false;
        self.seg_turn = 0;
        self.chain_put = None;
        self.chained = false;
        self.owed = false;
        let t0 = me.relay_stats.t0();
        Stats::bump(&me.relay_stats.turns);
        // ★ 2026-10-14 batching (doc §9): one CLAIM relays the covered entries one after another
        // (`user_loop::chain_more`: bounded by a count; a gate, a staged segment, a migration, a credit
        // shortage or a retry ends the chain) and rings the host doorbell ONCE at its end.
        let mut out;
        let mut submitted = false;
        let mut iterations = 0u32;
        loop {
            self.split = false;
            out = self.turn_body(me, r);
            iterations += 1;
            if let Ok(s) = &out {
                submitted |= *s;
            }
            let split = self.split || self.segment.is_some() || self.shadow.is_some();
            if !(out.is_ok()
                && kf_chan::user_loop::chain_more(iterations, self.again && !split, split))
            {
                break;
            }
            self.chained = true;
            self.again = false;
        }
        if out.is_ok() {
            out = Ok(submitted);
        }
        // whatever the chain ended with, the doorbell for what it published is rung (an error
        // already in hand wins over a doorbell error)
        let rung = self.ring_owed(me, r);
        if out.is_ok() {
            if let Err(e) = rung {
                out = Err(e);
            }
        }
        me.relay_stats.stage(kf_chan::user_loop::stage::TURN, t0);
        me.relay_stats.record_turn(self.seg_turn);
        match &out {
            Ok(_) if self.again => self.need = Need::Again,
            Err(Failure::Retry) => self.need = Need::Contended,
            _ => {}
        }
        out
    }

    fn turn_body(&mut self, me: &ChanPlane, r: &mut userrelay::Record) -> Turn<bool> {
        use kf_chan::user_loop::Need;
        if self.trace.as_ref().is_some_and(|t| !t.rereads.is_empty()) {
            // Measurement only: the deferred re-reads of `KF3_USER_RING_TRACE` run on the worker's
            // trace refresh (`ChanPlane::user_ring_refresh`), never in production.
            let software = software_snapshot(me, r).ok().flatten();
            self.trace_due(me, r, &software);
        }
        if let Some(t) = self.trace.as_mut() {
            t.last_turn = std::time::Instant::now();
            t.idle_reported = false;
        }
        if self.trace.is_some() {
            self.trace_stall(me, r);
        }
        let native = r.chan.ok_or("USER_RING_NATIVE_ABSENT")?;
        if !self.chained {
            let t_obs = me.relay_stats.t0();
            let (get, reference) = {
                let storage = r
                    .lease
                    .storage
                    .lock()
                    .map_err(|_| "USER_RING_STORAGE_POISONED")?;
                (storage.gpu_get()?, storage.gpu_reference()?)
            };
            me.relay_stats.stage(kf_chan::user_loop::stage::OBSERVE, t_obs);
            self.marks.observe(reference);
            for (generation, get) in self.fetch.observe(get).map_err(str::to_owned)? {
                if generation == self.generation {
                    self.guest_get = Some((generation, get));
                }
            }
            if let Some((generation, get)) = self.guest_get {
                if generation == self.generation {
                    // Logical memory access revalidates Free/reuse and RAM topology.
                    let t_wb = me.relay_stats.t0();
                    let wrote = r.userd.write(
                        me,
                        r.key,
                        &r.allocation,
                        kf_abi::submit::USERD_GP_GET,
                        &get.to_le_bytes(),
                    );
                    if wrote.is_ok() {
                        me.relay_stats.stage(kf_chan::user_loop::stage::GET_WRITEBACK, t_wb);
                        Stats::bump(&me.relay_stats.get_writebacks);
                    }
                    match wrote {
                        Ok(()) => self.guest_get = None,
                        Err(kf_rm::memory_list::AccessError::Busy) => {
                            // an admission lock is busy: look again (spin, then the safety net)
                            self.need = Need::Contended;
                            return Ok(false);
                        }
                        Err(e) => return Err(format!("USER_RING_GET_WRITE {e:?}").into()),
                    }
                } else {
                    self.guest_get = None;
                }
            }
        }
        if self.segment.is_none() {
            if me.inbox.walk_busy() {
                me.inbox.wait_walk(r.idx);
                return Ok(false);
            }
            // Doorbells counted BEFORE the PUT read (measurement only today).
            let doorbells = me.queued(r.idx);
            let put = match self.chain_put {
                Some((generation, put)) if generation == self.generation => put,
                _ => {
                    let mut put = [0u8; 4];
                    let t_put = me.relay_stats.t0();
                    Stats::bump(&me.relay_stats.put_reads);
                    r.userd
                        .read(
                            me,
                            r.key,
                            &r.allocation,
                            kf_abi::submit::USERD_GP_PUT,
                            &mut put,
                        )
                        .map_err(|e| match e {
                            // The USERD's allocation is no longer the channel's current one: the
                            // guest's free is retiring this relay (`retire_user_ring_token` cancels
                            // the record next). Not a refusal -- nothing was read or published; the
                            // look-again budget bounds it.
                            kf_rm::memory_list::AccessError::Gone => Failure::Retry,
                            e => access_error(e, "USER_RING_PUT_READ"),
                        })?;
                    me.relay_stats.stage(kf_chan::user_loop::stage::PUT_READ, t_put);
                    let put = u32::from_le_bytes(put);
                    if put < r.source.1 {
                        self.chain_put = Some((self.generation, put));
                    }
                    put
                }
            };
            if put >= r.source.1 {
                return Err("USER_RING_PUT_BOUND".into());
            }
            if put == self.cursor {
                return self.end_batch(me, r);
            }
            me.relay_stats.record_delta(put.wrapping_sub(self.cursor) & (r.source.1 - 1));
            if let Some(t) = self.trace.as_mut() {
                t.note_put(doorbells);
            }
            // Declaration admission only, not a pin. Native never fetches the
            // guest ring; EACH current GP/PB still has fresh exact source guards.
            let ring = if self.source_admitted {
                None
            } else {
                Some(
                    userread::mapped(
                        &r.mirror.rows,
                        &r.mirror.log,
                        me.ram,
                        self.ram_generation,
                        r.source.0,
                        u64::from(r.source.1) * 8,
                        store_read_bound(me.layout.carve()),
                    )
                    .map_err(|e| source_error(e, "USER_RING_WHOLE_RING"))?,
                )
            };
            let va = u64::from(self.cursor)
                .checked_mul(8)
                .and_then(|off| r.source.0.checked_add(off))
                .ok_or("USER_RING_GP_ADDRESS")?;
            let t_gp = me.relay_stats.t0();
            let source = self.snapshot(me, r, va, 8)?;
            me.relay_stats.stage(kf_chan::user_loop::stage::GP_SNAPSHOT, t_gp);
            let entry = u64::from_le_bytes(
                source
                    .bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| "USER_RING_GP_SNAPSHOT_SIZE")?,
            );
            if self.shadow.is_some() {
                return self.shadow_entry(me, r, ring, source, entry);
            }
            let descriptor = kf_chan::user_stream::inspect_entry(entry, userread::MAX_BYTES as u64)
                .map_err(|e| {
                    format!(
                        "USER_RING_GP_FORMAT {e:?} entry={entry:#018x} source_va={va:#x} cursor={:#x} put={put:#x} entries={}",
                        self.cursor, r.source.1
                    )
                })?;
            let guest_get =
                self.cursor.checked_add(1).ok_or("USER_RING_CURSOR")? & (r.source.1 - 1);
            // ★★ 2026-10-14 — GP_ENTRY1_SYNC_WAIT (public dev_pbdma.ref "NV_PPBDMA_GP_ENTRY1_SYNC":
            // Host does not begin fetching this entry's segment — and, for a LENGTH-0 entry, any
            // later one — until the preceding segment has been PROCESSED). A guest may therefore
            // publish entries behind a SYNC_WAIT whose pushbuffers it fills only later, gated by a
            // semaphore acquire in the preceding segment (measured, Win11 + 580.88: every scan
            // refusal and stale read sat behind an unfetched SYNC_WAIT NOP; the bytes were filled
            // within ms). Reading them early scans bytes the GPU will never fetch. So: before
            // touching anything at or after a SYNC_WAIT entry, author the same wait on our own
            // ring (barrier NOP + witness, exactly as staged segments do) and read on only once
            // the witness has begun processing — i.e. once the hardware itself would fetch.
            // ★ 2026-10-14 (MEASURED, `kf-syncwait-stress --families barrier`): the proof that the
            // barrier's wait finished is the marker's `USERD.REFERENCE` write, NOT `GP_GET` -- the
            // PBDMA pops GP entries ahead of processing (with an acquire stalling entry 0 `GP_GET`
            // already covered all three entries and the segment behind the barrier had not run).
            // A SYNC_WAIT entry reached with a packet OPEN across it has no witness (no byte may
            // enter an open packet) and the hardware does not delay it either (the split packet's
            // header counts the preceding segment as processed): it is read at once and counted.
            let t_gate = me.relay_stats.t0();
            let verdict = if entry >> 63 & 1 == 1 && self.scan.carried().is_some() {
                Stats::bump(&me.relay_stats.carried_ungated);
                if self.sync_carried < 4 {
                    self.sync_carried += 1;
                    eprintln!(
                        "kf3: USER_RING_SYNC_CARRIED {:#x}:{:#x} generation={} cursor={} entry={entry:#018x} open_packet={} (#{}) — a SYNC_WAIT entry behind an open packet cannot be gated (no witness fits inside a packet); read at once",
                        r.key.0,
                        r.key.1,
                        self.generation,
                        self.cursor,
                        carried_text(self.scan.carried()),
                        self.sync_carried
                    );
                } else {
                    self.sync_carried = self.sync_carried.saturating_add(1);
                }
                kf_chan::user_gate::Gate::Proceed
            } else {
                self.sync
                    .entry(entry, self.generation, self.cursor, self.marks.seen())
            };
            me.relay_stats.stage(kf_chan::user_loop::stage::GATE, t_gate);
            match verdict {
                kf_chan::user_gate::Gate::Proceed => {
                    // Measurement only: the SYNC_WAIT drain latency (gate authored -> marker processed).
                    if let Some(at) = self.gate_at.take() {
                        me.relay_stats.record_drain(
                            u64::try_from(at.elapsed().as_micros()).unwrap_or(u64::MAX),
                        );
                    }
                }
                kf_chan::user_gate::Gate::Wait => {
                    // The marker's interrupt announces it (its processing writes the reference).
                    self.need = Need::Marker {
                        target: self.gate_target,
                    };
                    return Ok(false);
                }
                kf_chan::user_gate::Gate::PublishBarrier => {
                    if self.fetch.available() < 2 {
                        return self.credit_wait(me, r);
                    }
                    // the barrier, then the marker (a packet is not open: the branch above)
                    let mut count = 0;
                    self.publish(
                        me,
                        r,
                        None,
                        ring.as_ref(),
                        Some(&source),
                        None,
                        |pump, storage| {
                            let (barrier, _) = pump.fetch.publish(None).map_err(str::to_owned)?;
                            storage.write_host_barrier(barrier)?;
                            let (slot, put) = pump.fetch.publish(None).map_err(str::to_owned)?;
                            let (n, value) = pump.marks.issue().map_err(str::to_owned)?;
                            count = n;
                            storage.write_host_marker(slot, value)?;
                            storage.publish_put(put)
                        },
                    )?;
                    self.trace_slots(|| {
                        vec![
                            "host barrier (control NOP, SYNC_WAIT)".into(),
                            format!("host marker #{count} (SET_REFERENCE + NON_STALL_INTERRUPT)"),
                        ]
                    });
                    self.source_admitted = true;
                    self.sync.published(count);
                    self.gate_target = count;
                    // the marker is this batch's marker too
                    self.unnotified = false;
                    self.marker_event = true;
                    self.gate_at = Some(std::time::Instant::now());
                    Stats::bump(&me.relay_stats.gates);
                    self.need = Need::Marker { target: count };
                    return Ok(true);
                }
            }
            if matches!(descriptor, kf_chan::user_stream::Source::Nop) {
                // one slot stays back for the batch marker
                if self.fetch.available() < 2 {
                    return self.credit_wait(me, r);
                }
                self.publish(
                    me,
                    r,
                    None,
                    ring.as_ref(),
                    Some(&source),
                    None,
                    |pump, storage| {
                        let (slot, put) = pump
                            .fetch
                            .publish(Some((pump.generation, guest_get)))
                            .map_err(str::to_owned)?;
                        storage.write_user_gp(slot, entry)?;
                        storage.publish_put(put)
                    },
                )?;
                let cursor = self.cursor;
                self.trace_slots(|| {
                    vec![format!("guest control cursor={cursor} entry={entry:#018x}")]
                });
                self.source_admitted = true;
                self.cursor = guest_get;
                self.unnotified = true;
                self.again = true;
                self.seg_turn += 1;
                return Ok(true);
            }
            let kf_chan::user_stream::Source::Segment(descriptor) = descriptor else {
                return Err("USER_RING_GP_FORMAT".into());
            };
            let t_pb = me.relay_stats.t0();
            let pb = self.snapshot(
                me,
                r,
                descriptor.gpu_va,
                usize::try_from(descriptor.len_bytes).map_err(|_| "USER_RING_PB_SIZE")?,
            )?;
            me.relay_stats.stage(kf_chan::user_loop::stage::PB_SNAPSHOT, t_pb);
            me.relay_stats.record_segment(pb.bytes.len() / 4);
            let mut words = Vec::new();
            words
                .try_reserve_exact(pb.bytes.len() / 4)
                .map_err(|_| "USER_RING_SCAN_ALLOC")?;
            for word in pb.bytes.chunks_exact(4) {
                words.push(u32::from_le_bytes(
                    word.try_into().map_err(|_| "USER_RING_WORD")?,
                ));
            }
            let software = software_snapshot(me, r)?;
            self.grace_was_retrying = self.grace.retrying();
            let t_scan = me.relay_stats.t0();
            let planned = stage_plan(&self.scan, &software, &words);
            me.relay_stats.stage(kf_chan::user_loop::stage::SCAN, t_scan);
            let planned = match planned {
                // ★ 2026-10-14: a scan refusal of a segment never published is re-read and rescanned
                // `REFUSE_RETRIES` times -- a COUNT, never a clock -- before it is final
                // (`kf_chan::user_gate::Grace`); the re-read is an immediate re-turn.
                Err(Failure::Refused(why))
                    if (why.starts_with("USER_RING_SCAN ")
                        || why == "USER_RING_TRIGGER_PARTIAL_SUFFIX_UNSUPPORTED")
                        && self.grace.refused(self.generation, self.cursor) =>
                {
                    if self.grace_reported < 4 && !self.grace_was_retrying {
                        self.grace_reported += 1;
                        eprintln!(
                            "kf3: USER_RING_SCAN_RETRY {:#x}:{:#x} cursor={} (re-read and rescan up to {} times before refusing): {why}",
                            r.key.0,
                            r.key.1,
                            self.cursor,
                            kf_chan::user_gate::REFUSE_RETRIES
                        );
                    }
                    self.again = true;
                    return Ok(false);
                }
                other => other,
            };
            if planned.is_ok() {
                self.grace.clear();
            }
            if self.trace.is_some() {
                match &planned {
                    Ok((_, rare)) => {
                        self.trace_segment(r, put, entry, descriptor, &pb, &words, *rare, &software)
                    }
                    Err(why) => self.trace_refusal(
                        me,
                        r,
                        (put, va, entry),
                        descriptor,
                        &pb,
                        &words,
                        &software,
                        why,
                    ),
                }
            }
            let (patched, rare) = planned?;
            self.segment = Some(Segment {
                ring,
                software,
                source,
                pb,
                entry,
                words,
                patched,
                at: 0,
                generation: self.generation,
                guest_get,
                rare,
                staged: false,
                rare_counted: false,
                wait: None,
            });
        }
        let mut segment = self.segment.take().ok_or("USER_RING_SEGMENT_ABSENT")?;
        let outcome = self.segment_turn(me, r, native, &mut segment);
        match outcome {
            Ok((submitted, finished)) => {
                if !finished {
                    // `segment_turn` said what it waits for (`self.need`); the segment stays
                    self.segment = Some(segment);
                }
                // A CPU-only action/receipt can expose a new source delta too.
                // Preserve a durable worker pass even with no new guest ring.
                self.again = finished || submitted;
                Ok(submitted)
            }
            Err(e) => {
                self.segment = Some(segment);
                Err(e)
            }
        }
    }

    /// ★ 2026-10-14 -- the batch marker (`kf_chan::user_loop`, design doc §3.3): one private segment
    /// `[NON_STALL_INTERRUPT, 0]` behind everything published so far. Its `GP_GET` passing is the
    /// proof every earlier entry began processing; its interrupt (the engine's non-stall event, in the
    /// worker's epoll) is the wake. No guest source is involved, so no source guard is taken.
    fn publish_marker(&mut self, me: &ChanPlane, r: &userrelay::Record) -> Turn<()> {
        let t_marker = me.relay_stats.t0();
        let out = self.publish_marker_body(me, r);
        me.relay_stats.stage(kf_chan::user_loop::stage::MARKER, t_marker);
        out
    }

    fn publish_marker_body(&mut self, me: &ChanPlane, r: &userrelay::Record) -> Turn<()> {
        self.publish(me, r, None, None, None, None, |pump, storage| {
            let (slot, put) = pump.fetch.publish(None).map_err(str::to_owned)?;
            let (_, value) = pump.marks.issue().map_err(str::to_owned)?;
            storage.write_host_marker(slot, value)?;
            storage.publish_put(put)
        })?;
        self.unnotified = false;
        self.marker_event = true;
        Stats::bump(&me.relay_stats.markers);
        Ok(())
    }

    /// The turn found `GP_PUT == cursor`. End the batch: a marker behind unannounced work when a
    /// packet boundary allows it (a packet the scan carries open owns the next dword -- no byte may
    /// enter), and wait for that marker to be PROCESSED: its interrupt is the wake, its reference
    /// the proof, and processing implies the pops the guest's `GP_GET` write-back needs. With no
    /// marker possible the relay waits for the pops themselves (spin / safety net).
    fn end_batch(&mut self, me: &ChanPlane, r: &userrelay::Record) -> Turn<bool> {
        use kf_chan::user_loop::Need;
        if self.unnotified {
            if self.scan.carried().is_some() {
                // no marker is possible: the wake for this work is the spin or the safety net
                self.unnotified = false;
                self.marker_event = false;
                self.need = Need::Popped {
                    target: self.fetch.issued(),
                };
                return Ok(false);
            }
            if self.fetch.available() >= 1 {
                self.publish_marker(me, r)?;
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
            // published work in flight: the last marker's processing frees the guest's ring (the
            // `GP_GET` write-back)
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

    /// The private ring has no credit for what the turn wants to publish. Put a marker behind the
    /// unannounced work out of the reserved slot first, so SOME interrupt announces progress.
    fn credit_need(&mut self, me: &ChanPlane, r: &userrelay::Record) -> Turn<()> {
        use kf_chan::user_loop::Need;
        let mut marker = self.marker_event;
        if self.unnotified {
            if self.scan.carried().is_none() && self.fetch.available() >= 1 {
                self.publish_marker(me, r)?;
                marker = true;
            } else {
                self.unnotified = false;
                self.marker_event = false;
                marker = false;
            }
        }
        Stats::bump(&me.relay_stats.credit_waits);
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

    fn credit_wait(&mut self, me: &ChanPlane, r: &userrelay::Record) -> Turn<bool> {
        self.credit_need(me, r)?;
        Ok(false)
    }

    /// No host verb inside any admission guard. The caller holds Record over
    /// the final mapped doorbell too; native Free takes that mutex off-drainer.
    /// Opaque diagnostic: observations never authorize host actions or retirement.
    fn shadow_entry(
        &mut self,
        me: &ChanPlane,
        r: &userrelay::Record,
        ring: Option<Mapping<'static>>,
        source: Snapshot<'static>,
        entry: u64,
    ) -> Turn<bool> {
        use super::usershadow::Class;
        if self.fetch.available() == 0 {
            self.need = kf_chan::user_loop::Need::Popped {
                target: self.fetch.fetched() + 1,
            };
            return Ok(false);
        }
        let mut shadow = self.shadow.take().ok_or("USER_RING_SHADOW_ABSENT")?;
        let mut events = Vec::new();
        let mut sample = Vec::new();
        match kf_chan::user_stream::inspect_entry(entry, userread::MAX_BYTES as u64) {
            Ok(kf_chan::user_stream::Source::Nop) => {}
            Ok(kf_chan::user_stream::Source::Segment(d)) => {
                if !shadow.can_observe() {
                    if shadow.record(Class::UnknownSuffix) {
                        events.push((
                            Class::UnknownSuffix,
                            "history/address interpretation unknown; no further CPU PB reads"
                                .into(),
                        ));
                    }
                } else if shadow.charge(d.len_bytes as usize) {
                    match self.snapshot(me, r, d.gpu_va, d.len_bytes as usize) {
                        Ok(pb) => {
                            let words =
                                super::usershadow::decode_words(&pb.bytes, userread::MAX_BYTES);
                            if let Err(why) = &words {
                                shadow.lose_history();
                                if shadow.record(Class::Budget) {
                                    events.push((Class::Budget, (*why).to_owned()));
                                }
                            }
                            if let Ok(words) = words {
                                let sw = software_snapshot(me, r);
                                match sw {
                                    Ok(sw) => shadow.observe(
                                        &words,
                                        |v| sw.as_ref().is_some_and(|s| s.matches(v)),
                                        |c, why| events.push((c, why)),
                                    ),
                                    Err(e) => {
                                        shadow.lose_history();
                                        if shadow.record(Class::Source) {
                                            events.push((
                                                Class::Source,
                                                format!("software identity {e:?}"),
                                            ));
                                        }
                                    }
                                }
                                if !events.is_empty() {
                                    sample.extend(words.iter().take(128).copied());
                                }
                            }
                        }
                        Err(e) => {
                            shadow.lose_history();
                            if shadow.record(Class::Source) {
                                events.push((Class::Source, format!("observation read {e:?}")));
                            }
                        }
                    }
                } else if shadow.record(Class::ObservationBudget) {
                    events.push((
                        Class::ObservationBudget,
                        "4MiB/channel observation cap; later semantic suffix unknown".into(),
                    ));
                }
            }
            Err(e) => {
                shadow.lose_history();
                let c = if e == kf_chan::user_stream::Refusal::Budget {
                    Class::Budget
                } else {
                    Class::Entry
                };
                if shadow.record(c) {
                    events.push((c, format!("GP {e:?}; no CPU dereference, suffix unknown")));
                }
            }
        }
        for (class, why) in events {
            eprintln!(
                "kf3: USER_RING_SHADOW {:x?} idx={} native={:x?} source_generation={} native_issued={} class={class:?} category={} GP={entry:#018x} cursor={} source_epoch={} {why}; sample_first128={sample:x?}; mutable observation NOT execution",
                r.key,
                r.idx,
                r.chan.map(|c| (c.chan, c.token)),
                self.generation,
                self.fetch.issued(),
                class.category(),
                self.cursor,
                source.epoch
            );
        }
        let guest_get = (self.cursor + 1) & (r.source.1 - 1);
        let publication = self.publish(
            me,
            r,
            None,
            ring.as_ref(),
            Some(&source),
            None,
            |pump, storage| {
                let (slot, put) = pump
                    .fetch
                    .publish(Some((pump.generation, guest_get)))
                    .map_err(str::to_owned)?;
                storage.write_shadow_gp(slot, entry)?;
                storage.publish_put(put)
            },
        );
        shadow.finish_publication(publication.is_ok());
        self.shadow = Some(shadow);
        publication?;
        self.source_admitted = true;
        self.cursor = guest_get;
        self.again = true;
        Ok(true)
    }

    fn publish(
        &mut self,
        me: &ChanPlane,
        r: &userrelay::Record,
        software: Option<&Option<Software>>,
        ring: Option<&Mapping<'static>>,
        gp: Option<&Snapshot<'static>>,
        pb: Option<&Snapshot<'static>>,
        write: impl FnOnce(
            &mut Self,
            &kf_chan::user_storage::Storage<kf_chan::user_storage::CpuView>,
        ) -> Result<(), String>,
    ) -> Turn<()> {
        if me.inbox.walk_busy() {
            me.inbox.wait_walk(r.idx);
            return Err(Failure::Retry);
        }
        let t_pub = me.relay_stats.t0();
        let userd = r.userd.clone();
        userd
            .admit(|| -> Turn<()> {
                let mut pt = locked(me.pt.try_lock(), "USER_RING_PT_POISONED")?;
                let t = pt
                    .get_mut(&r.key)
                    .filter(|t| t.user_ring && Arc::ptr_eq(&t.allocation, &r.allocation))
                    .ok_or("USER_RING_ALLOCATION_GONE")?;
                if software.is_some_and(|expected| !same_software(expected, &t.user_defapi)) {
                    return Err(Failure::Retry);
                }
                if let Some(sw) = &t.user_defapi {
                    if !sw.live.load(Ordering::Acquire) {
                        return Err("USER_RING_SOFTWARE_OBJECT_GONE".into());
                    }
                    if !sw.ready.load(Ordering::Acquire) {
                        return Err(Failure::Retry);
                    }
                }
                if t.stopped || t.disabled {
                    return Err(Failure::Retry);
                }
                if me.stop.load(Ordering::Acquire) {
                    return Err("USER_RING_PUBLICATION_STOPPED".into());
                }
                let storage = locked(r.lease.storage.try_lock(), "USER_RING_STORAGE_POISONED")?;
                // Exclude first-object admission even on an uncertain native write.
                // A premature/hostile doorbell can only close its own admission.
                let publish = || {
                    me.ram
                        .admit_user_generation(self.ram_generation, || {
                            t.user_published = true;
                            write(self, &storage).map_err(Failure::from)
                        })
                        .map_err(|e| source_error(e, "USER_RING_PUBLICATION_TOPOLOGY"))?
                };
                let sources = || {
                    if let Some(gp) = gp {
                        gp.admit(|| {
                            if let Some(pb) = pb {
                                pb.admit(publish)
                                    .map_err(|e| source_error(e, "USER_RING_PB_ADMISSION"))?
                            } else {
                                publish()
                            }
                        })
                        .map_err(|e| source_error(e, "USER_RING_GP_ADMISSION"))?
                    } else {
                        publish()
                    }
                };
                if let Some(ring) = ring {
                    ring.admit(sources)
                        .map_err(|e| source_error(e, "USER_RING_WHOLE_RING_ADMISSION"))?
                } else {
                    sources()
                }
            })
            .map_err(|e| access_error(e, "USER_RING_USERD_ADMISSION"))??;
        Stats::bump(&me.relay_stats.publications);
        // ★ 2026-10-14 batching: the PUT store is committed; the host doorbell for it (and for every
        // publication the claim makes after it) is rung ONCE, at the end of the chain.
        self.owed = true;
        me.relay_stats.stage(kf_chan::user_loop::stage::PUBLISH, t_pub);
        Ok(())
    }

    /// Ring the host doorbell this claim owes for what it published (`publish` only stores the PUT).
    /// Called by `turn` on every way out of a chain: published work is never left without its bell.
    fn ring_owed(&mut self, me: &ChanPlane, r: &userrelay::Record) -> Turn<()> {
        if !self.owed {
            return Ok(());
        }
        self.owed = false;
        let t_bell = me.relay_stats.t0();
        Stats::bump(&me.relay_stats.host_doorbells);
        let rung = me
            .rm
            .doorbell(r.chan.ok_or("USER_RING_NATIVE_ABSENT")?.token)
            .map_err(|e| format!("USER_RING_DOORBELL_UNCERTAIN {e:?}").into());
        me.relay_stats.stage(kf_chan::user_loop::stage::DOORBELL, t_bell);
        rung
    }

    fn segment_turn(
        &mut self,
        me: &ChanPlane,
        r: &mut userrelay::Record,
        _native: kf_host::Channel,
        s: &mut Segment,
    ) -> Turn<(bool, bool)> {
        if let Some(mut wait) = s.wait.take() {
            if self.marks.seen() < wait.witness {
                let target = wait.witness;
                s.wait = Some(wait);
                // the staged witness is a marker: its processing writes the reference, its
                // interrupt is the wake
                self.need = kf_chan::user_loop::Need::Marker { target };
                return Ok((false, false));
            }
            if let Some(trigger) = wait.trigger {
                match self.software(me, r, trigger.call, &mut wait.action) {
                    Ok(true) => {}
                    Ok(false) => {
                        s.wait = Some(wait);
                        return Ok((false, false));
                    }
                    Err(e) => {
                        s.wait = Some(wait);
                        return Err(e);
                    }
                }
                if wait.gapless {
                    // The suffix went out with the prefix: the segment is done.
                    s.at = s.words.len();
                    return Ok((false, true));
                }
                let (at, finished) = suffix_after_trigger(s.at, trigger, s.words.len())?;
                s.at = at;
                // The prefix witness already retired every published byte. A
                // trigger at END has no suffix to consume: EVICT can deschedule
                // the channel, so never append a new receipt it cannot fetch.
                return Ok((false, finished));
            }
            // This receipt retires private staging only. Guest GP_GET follows
            // the first representing descriptor's real fetch, independently.
            return Ok((false, true));
        }
        let mut next = self.scan.clone();
        let trigger = next
            .segment(s.words.get(s.at..).ok_or("USER_RING_SCAN_BOUND")?, |v| {
                s.software.as_ref().is_some_and(|sw| sw.matches(v))
            })
            .map_err(|e| format!("USER_RING_SCAN {e:?}"))?;
        if !s.rare {
            // one slot stays back for the batch marker
            if self.fetch.available() < 2 {
                self.credit_need(me, r)?;
                return Ok((false, false));
            }
            self.publish(
                me,
                r,
                Some(&s.software),
                s.ring.as_ref(),
                Some(&s.source),
                Some(&s.pb),
                |pump, storage| {
                    let (slot, put) = pump
                        .fetch
                        .publish(Some((s.generation, s.guest_get)))
                        .map_err(str::to_owned)?;
                    storage.write_user_gp(slot, s.entry)?;
                    storage.publish_put(put)
                },
            )?;
            let (cursor, entry, n) = (self.cursor, s.entry, s.words.len());
            let (cin, cout) = (self.scan.carried(), next.carried());
            self.trace_slots(|| {
                vec![format!(
                    "guest segment cursor={cursor} entry={entry:#018x} words={n} carried_in={} carried_out={}",
                    carried_text(cin),
                    carried_text(cout)
                )]
            });
            self.source_admitted = true;
            self.scan = next;
            self.cursor = s.guest_get;
            self.unnotified = true;
            self.seg_turn += 1;
            return Ok((true, true));
        }
        let mut end = trigger.map_or(s.words.len(), |t| s.at.saturating_add(t.start));
        let mut gapless = false;
        if let Some(t) = trigger
            && self.trigger_is_channel_update(me, r, t.call)
            && let Some((all, tail)) = gapless_remainder(&next, &s.words, s.at, t, |v| {
                s.software.as_ref().is_some_and(|sw| sw.matches(v))
            })
        {
            end = all;
            next = tail;
            gapless = true;
        }
        let bytes = end
            .checked_sub(s.at)
            .and_then(|n| n.checked_mul(4))
            .ok_or("USER_RING_PREFIX_BOUND")?;
        let credits = if bytes == 0 { 2 } else { 3 };
        if self.fetch.available() < credits {
            self.credit_need(me, r)?;
            return Ok((false, false));
        }
        // Record excludes native Free, and a previous segment cannot retire
        // until its real HOST witness has fetched. Populate unsubmitted private
        // bytes before taking shared source/topology/Pt publication guards.
        // A retry leaves PUT unchanged; re-staging those bytes is harmless.
        if !s.staged {
            let storage = locked(r.lease.storage.try_lock(), "USER_RING_STORAGE_POISONED")?;
            storage.write_authored(0, &s.patched)?;
        }
        // Initial publication retains exact source guards. Once staged, the owned
        // snapshot survives the guest freeing/moving its old ring at migration.
        let gp = (!s.staged).then_some(&s.source);
        let pb = (!s.staged).then_some(&s.pb);
        self.publish(
            me,
            r,
            Some(&s.software),
            if s.staged { None } else { s.ring.as_ref() },
            gp,
            pb,
            |pump, storage| {
                if bytes != 0 {
                    let source = source_receipt(s.staged, true, s.generation, s.guest_get);
                    let (slot, _) = pump.fetch.publish(source).map_err(str::to_owned)?;
                    storage.write_gp_level(
                        slot,
                        (s.at as u64)
                            .checked_mul(4)
                            .ok_or("USER_RING_PREFIX_BOUND")?,
                        bytes as u64,
                        s.entry & (1 << 63) != 0,
                        s.entry & (1 << 41) != 0,
                    )?;
                }
                let source = source_receipt(s.staged, bytes == 0, s.generation, s.guest_get);
                let (barrier, _) = pump.fetch.publish(source).map_err(str::to_owned)?;
                storage.write_host_barrier(barrier)?;
                let (witness, put) = pump.fetch.publish(None).map_err(str::to_owned)?;
                let (_, value) = pump.marks.issue().map_err(str::to_owned)?;
                storage.write_host_marker(witness, value)?;
                storage.publish_put(put)
            },
        )?;
        let (at, entry, staged) = (s.at, s.entry, s.staged);
        self.trace_slots(|| {
            let mut v = Vec::new();
            if bytes != 0 {
                v.push(format!(
                    "staged copy entry={entry:#018x} from word {at} bytes={bytes} restaged={staged}"
                ));
            }
            v.push("host barrier (control NOP, SYNC_WAIT)".into());
            v.push("host witness Bytes (staged; batch marker)".into());
            v
        });
        if !s.staged && s.generation == self.generation {
            self.source_admitted = true;
            self.cursor = s.guest_get;
        }
        s.staged = true;
        if !s.rare_counted {
            s.rare_counted = true;
            self.seg_turn += 1;
        }
        self.scan = next;
        s.wait = Some(Wait {
            witness: self.marks.issued(),
            trigger,
            action: None,
            gapless,
        });
        Ok((true, false))
    }

    /// Whether `call` names this channel's registered `FIFO_UPDATE_CHANNEL_INFO` (a peek: the
    /// registry counts a lookup, nothing else; the action itself still runs through [`Self::software`]).
    fn trigger_is_channel_update(
        &self,
        me: &ChanPlane,
        r: &userrelay::Record,
        call: kf_chan::tmode::SwCall,
    ) -> bool {
        let Some(reg) = me.defapi_reg.get() else {
            return false;
        };
        let Ok(Some(sw)) = software_snapshot(me, r) else {
            return false;
        };
        if !sw.matches(call.value) {
            return false;
        }
        let key = kf_rm::defapi::ObjKey {
            client: r.key.0,
            object: sw.guest,
        };
        reg.lookup(key, call.data)
            .is_ok_and(|e| matches!(e.decoded, kf_abi::defapi::Bundle::UpdateChannelInfo(_)))
    }

    fn software(
        &mut self,
        me: &ChanPlane,
        r: &mut userrelay::Record,
        call: kf_chan::tmode::SwCall,
        pending: &mut Option<PendingAction>,
    ) -> Turn<bool> {
        let reg = me.defapi_reg.get().ok_or("USER_RING_REGISTRY_ABSENT")?;
        if let Some(action) = pending.take() {
            let (plan, result, flushed) = match action {
                PendingAction::Invalidate { ticket, plan } => match me.inbox.split_result(ticket) {
                    Some(result) => (plan, result, true),
                    None => {
                        *pending = Some(PendingAction::Invalidate { ticket, plan });
                        return Ok(false);
                    }
                },
                PendingAction::Context { cell, plan } => {
                    let result = cell.try_lock().ok().and_then(|mut result| result.take());
                    match result {
                        Some(result) => (plan, result.map(|_| ()), false),
                        None => {
                            *pending = Some(PendingAction::Context { cell, plan });
                            return Ok(false);
                        }
                    }
                }
            };
            reg.executed_entry(plan.key, &plan.entry, flushed && result.is_ok());
            result?;
            return Ok(true);
        }
        let sw = software_snapshot(me, r)?
            .filter(|sw| sw.matches(call.value))
            .ok_or("USER_RING_SOFTWARE_OBJECT_GONE")?;
        let handle = sw.guest;
        let key = kf_rm::defapi::ObjKey {
            client: r.key.0,
            object: handle,
        };
        let entry = reg
            .lookup(key, call.data)
            .map_err(|e| format!("USER_RING_UNREGISTERED_API {e:#x}"))?;
        let plan = crate::defapi::Planned {
            key,
            bundle: entry.decoded,
            entry,
        };
        me.defapi_triggers.fetch_add(1, Ordering::Relaxed);
        let outcome = match plan.bundle {
            kf_abi::defapi::Bundle::UpdateChannelInfo(update) => self.migrate(me, r, update),
            kf_abi::defapi::Bundle::InvalidateTlb { .. } => {
                let ticket =
                    me.inbox
                        .request_user_split(r.idx, r.vas_key)
                        .map_err(|e| match e {
                            crate::mem::UserSplitError::Busy => Failure::Retry,
                            crate::mem::UserSplitError::Refused(why) => why.into(),
                        })?;
                *pending = Some(PendingAction::Invalidate { ticket, plan });
                return Ok(false);
            }
            kf_abi::defapi::Bundle::InitializeCtx {
                chan_client,
                object,
                ..
            }
            | kf_abi::defapi::Bundle::PromoteCtx {
                chan_client,
                object,
                ..
            }
            | kf_abi::defapi::Bundle::EvictCtx {
                chan_client,
                object,
                ..
            } => {
                let pt = locked(me.pt.try_lock(), "USER_RING_PT_POISONED")?;
                let own = pt.get(&r.key).is_some_and(|t| {
                    Arc::ptr_eq(&t.allocation, &r.allocation)
                        && chan_client == r.key.0
                        && (object == r.key.1 || t.tsg == Some(object))
                });
                drop(pt);
                if !own {
                    Err("USER_RING_CONTEXT_FOREIGN_TARGET".into())
                } else {
                    let cell = Arc::new(Mutex::new(None));
                    me.user_ring_ctx_act(
                        plan.clone(),
                        cell.clone(),
                        r.idx,
                        r.key,
                        r.allocation.clone(),
                    )?;
                    *pending = Some(PendingAction::Context { cell, plan });
                    return Ok(false);
                }
            }
            _ => Err(format!("USER_RING_DEFERRED_UNSUPPORTED {:?}", plan.bundle).into()),
        };
        if outcome != Err(Failure::Retry) {
            reg.executed_entry(plan.key, &plan.entry, false);
        }
        outcome?;
        Ok(true)
    }

    fn migrate(
        &mut self,
        me: &ChanPlane,
        r: &mut userrelay::Record,
        u: kf_abi::defapi::FifoUpdate,
    ) -> Turn<()> {
        if (u.client, u.channel) != r.key
            || u.userd_memory == 0
            || u.userd_offset & 0x1ff != 0
            || u.gpfifo_va & 7 != 0
            || u.entries < 2
            || !u.entries.is_power_of_two()
            || u.entries > 1 << 20
        {
            return Err("USER_RING_MIGRATION_DECLARATION".into());
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or("USER_RING_SOURCE_SEQUENCE")?;
        if self.trace.is_some() {
            // Measurement only: the OLD ring at the switch (our cursor, the guest's PUT/GET, the
            // entries around our cursor), to compare with the new ring's first entries.
            let mut word = |off: u64| {
                let mut b = [0u8; 4];
                r.userd
                    .read(me, r.key, &r.allocation, off, &mut b)
                    .ok()
                    .map(|()| u32::from_le_bytes(b))
            };
            let (put, get) = (
                word(kf_abi::submit::USERD_GP_PUT),
                word(kf_abi::submit::USERD_GP_GET),
            );
            let entries = r.source.1;
            let mut around = Vec::new();
            if entries.is_power_of_two() && entries >= 2 {
                for k in 0..12u32 {
                    let idx = self
                        .cursor
                        .wrapping_add(entries)
                        .wrapping_sub(4)
                        .wrapping_add(k)
                        & (entries - 1);
                    let e = self
                        .snapshot(me, r, r.source.0 + u64::from(idx) * 8, 8)
                        .ok()
                        .and_then(|s| s.bytes.as_slice().try_into().ok().map(u64::from_le_bytes));
                    around.push((idx, e));
                }
            }
            eprintln!(
                "kf3: USER_RING_TRACE MIGRATING {:#x}:{:#x} old ring {:#x}x{} our_cursor={} guest put={put:?} get={get:?} entries_around_cursor={around:x?}",
                r.key.0, r.key.1, r.source.0, entries, self.cursor
            );
        }
        let userd = me
            .memory_access
            .lease(u.client, u.userd_memory, u.userd_offset, 0x200)
            .map_err(|e| access_error(e, "USER_RING_MIGRATION_USERD"))?;
        let ring = userread::mapped(
            &r.mirror.rows,
            &r.mirror.log,
            me.ram,
            self.ram_generation,
            u.gpfifo_va,
            u64::from(u.entries) * 8,
            store_read_bound(me.layout.carve()),
        )
        .map_err(|e| source_error(e, "USER_RING_MIGRATION_RING"))?;
        if me.inbox.walk_busy() {
            me.inbox.wait_walk(r.idx);
            return Err(Failure::Retry);
        }
        userd
            .admit(|| -> Turn<()> {
                let mut pt = locked(me.pt.try_lock(), "USER_RING_PT_POISONED")?;
                let t = pt
                    .get_mut(&r.key)
                    .filter(|t| t.user_ring && Arc::ptr_eq(&t.allocation, &r.allocation))
                    .ok_or("USER_RING_ALLOCATION_GONE")?;
                ring.admit(|| {
                    // New source starts at0. Distinct-slot native reference and
                    // Windows1630's two growth transitions support this path;
                    // arbitrary seeded cursors remain unmeasured. Never reset
                    // the private native GET, whose fetch credits continue.
                    r.userd = userrelay::UserdSource::Registered(userd.clone());
                    r.source = (u.gpfifo_va, u.entries);
                    t.ring_at = (u.gpfifo_va, u.entries, None);
                    self.generation = generation;
                    self.source_admitted = false;
                    self.cursor = 0;
                    self.grace.clear();
                    self.guest_get = None;
                })
                .map_err(|e| source_error(e, "USER_RING_MIGRATION_ADMISSION"))
            })
            .map_err(|e| access_error(e, "USER_RING_MIGRATION_USERD_ADMISSION"))??;
        eprintln!(
            "kf3: USER_RING_MIGRATED {:#x}:{:#x} source={:#x}x{} generation={generation}; private native ring/USERD unchanged; no host migration verb",
            r.key.0, r.key.1, u.gpfifo_va, u.entries
        );
        if self.trace.is_some() {
            // Measurement only: what the guest's NEW ring holds at the switch.
            let mut word = |off: u64| {
                let mut b = [0u8; 4];
                r.userd
                    .read(me, r.key, &r.allocation, off, &mut b)
                    .ok()
                    .map(|()| u32::from_le_bytes(b))
            };
            let (put, get) = (
                word(kf_abi::submit::USERD_GP_PUT),
                word(kf_abi::submit::USERD_GP_GET),
            );
            let mut first = Vec::new();
            for i in 0..16u64 {
                first.push(
                    self.snapshot(me, r, u.gpfifo_va + i * 8, 8)
                        .ok()
                        .and_then(|s| s.bytes.as_slice().try_into().ok().map(u64::from_le_bytes)),
                );
            }
            eprintln!(
                "kf3: USER_RING_TRACE MIGRATED {:#x}:{:#x} new USERD put={put:?} get={get:?} first_entries={first:x?} fetch_issued={} fetch_fetched={} sync_waiting={}",
                r.key.0,
                r.key.1,
                self.fetch.issued(),
                self.fetch.fetched(),
                self.sync.waiting()
            );
        }
        Ok(())
    }
}

impl ChanPlane {
    pub(super) fn user_ring_software_object(
        &self,
        client: u32,
        parent: u32,
        handle: u32,
        notify_completion: bool,
    ) -> ChanAnswer {
        let key = (client, parent);
        let admission = (|| -> Result<_, String> {
            let mut pt = self.pt.lock().map_err(|_| "USER_RING_PT_POISONED")?;
            let t = pt
                .get_mut(&key)
                .filter(|t| t.user_ring)
                .ok_or("USER_RING_ALLOCATION_GONE")?;
            // Match CPU-RM statement-time numbering even when this alloc refuses.
            let number = t.sw_ids.register().ok_or("USER_RING_SW_NUMBER_EXHAUSTED")?;
            if notify_completion {
                return Err("USER_RING_DEFERRED_NOTIFICATION_UNSUPPORTED".into());
            }
            if t.user_published {
                return Err("USER_RING_LATE_SOFTWARE_OBJECT_UNSUPPORTED".into());
            }
            if number != 1 || t.user_defapi.is_some() {
                return Err("USER_RING_FIRST_SOFTWARE_OBJECT_ONLY".into());
            }
            let sw = Software {
                guest: handle,
                number,
                live: Arc::new(AtomicBool::new(true)),
                ready: Arc::new(AtomicBool::new(false)),
            };
            t.user_defapi = Some(sw.clone());
            Ok((t.allocation.clone(), t.chan, sw))
        })();
        let (allocation, native, sw) = match admission {
            Ok(v) => v,
            Err(why) => {
                return ChanAnswer::Refused {
                    status: NV_ERR_NOT_SUPPORTED,
                    why,
                };
            }
        };
        let d = kf_gsp::Deferred::new();
        let abandoned = sw.live.clone();
        d.on_orphaned(move || {
            abandoned.store(false, Ordering::Release);
        });
        let act: Act = Box::new(move |me| {
            let fail = |why: &str| (NV_ERR_INVALID_STATE, why.to_owned());
            let record = me
                .user_ring_record(native.token)
                .ok_or_else(|| fail("USER_RING_RECORD_GONE"))?;
            // Off-drainer: exclude publication and native Free through this RM verb.
            let r = record
                .lock()
                .map_err(|_| fail("USER_RING_RECORD_POISONED"))?;
            if r.free_attempted
                || r.retired
                || !Arc::ptr_eq(&r.allocation, &allocation)
                || !sw.live.load(Ordering::Acquire)
            {
                return Err(fail("USER_RING_SW_ADMISSION_GONE"));
            }
            let current = |pt: &PtMap| {
                pt.get(&key).is_some_and(|t| {
                    Arc::ptr_eq(&t.allocation, &allocation)
                        && t.chan == native
                        && t.user_defapi
                            .as_ref()
                            .is_some_and(|v| Arc::ptr_eq(&v.live, &sw.live))
                })
            };
            if !me.pt.lock().is_ok_and(|pt| current(&pt)) {
                return Err(fail("USER_RING_SW_ALLOCATION_GONE"));
            }
            // No host deferred entries: native SET_OBJECT/NOP is supported; all
            // intercepted host actions use the separately owned guest registry.
            let host = me.rm.alloc_deferred_api(native).map_err(|e| {
                sw.live.store(false, Ordering::Release);
                (NV_ERR_NOT_SUPPORTED, format!("USER_RING_NATIVE_5080 {e:?}"))
            })?;
            let mut pt = me.pt.lock().map_err(|_| fail("USER_RING_PT_POISONED"))?;
            if !current(&pt) || !sw.live.load(Ordering::Acquire) {
                // Retain the native child under its exact channel until Free.
                return Err(fail("USER_RING_SW_ALLOCATION_GONE_AFTER_NATIVE_ALLOC"));
            }
            pt.get_mut(&key)
                .ok_or_else(|| fail("USER_RING_ALLOCATION_GONE"))?
                .defapi_host = Some((handle, host));
            sw.ready.store(true, Ordering::Release);
            drop(pt);
            drop(r);
            me.user_ring_wake(native.token);
            Ok(format!(
                "USER_RING_5080 admitted own {client:#x}:{handle:#x}, classID{} host {host:#x}; no host registrations",
                sw.number
            ))
        });
        self.defer_cell("USER ring software object", act, d)
    }

    pub(super) fn user_ring_serve(&self, ht: u32) -> Option<bool> {
        let t_serve = self.relay_stats.t0();
        let record = self.user_ring_record(ht)?;
        let Ok(mut r) = record.enter() else {
            // the act thread holds the record: `retry` stays set (its completion rings; the spin
            // looks at the demand a bounded number of times, then only the safety net does)
            record.contended_note();
            return Some(false);
        };
        if r.retired || r.free_attempted || r.dead.is_some() {
            record.cancel();
            return Some(false);
        }
        let pt = match self.pt.try_lock() {
            Ok(pt) => pt,
            Err(std::sync::TryLockError::WouldBlock) => {
                record.contended_note();
                return Some(false);
            }
            Err(std::sync::TryLockError::Poisoned(_)) => {
                record.cancel();
                r.dead = Some("USER_RING_PT_POISONED".into());
                self.poisoned.fetch_add(1, Ordering::Relaxed);
                return Some(false);
            }
        };
        let Some(t) = pt
            .get(&r.key)
            .filter(|t| t.user_ring && Arc::ptr_eq(&t.allocation, &r.allocation))
        else {
            record.cancel();
            return Some(false);
        };
        if t.stopped || t.disabled {
            // An enable re-requests it (`user_ring_wake`); there is nothing to look for meanwhile.
            record.retry.store(false, Ordering::Release);
            return Some(false);
        }
        if self.stop.load(Ordering::Acquire) {
            record.retry.store(false, Ordering::Release);
            return Some(false);
        }
        drop(pt);
        let woke = record.retry.swap(false, Ordering::AcqRel);
        let mut pump = match r.pump.take() {
            Some(p) => p,
            None => return Some(false),
        };
        if let Some(t) = pump.trace.as_mut() {
            t.wake(woke);
        }
        let outcome = pump.turn(self, &mut r);
        r.pump = Some(pump);
        self.relay_stats.stage(kf_chan::user_loop::stage::SERVE, t_serve);
        match outcome {
            Ok(submitted) => {
                self.user_ring_arm(&record, &mut r);
                Some(submitted)
            }
            Err(Failure::Retry) => {
                if let Some(p) = &mut r.pump {
                    // A changed/contended prepublication snapshot has no native
                    // credit. Re-read it rather than retrying a permanently stale
                    // view. Staged native work and its receipt state stay owned.
                    if p.segment.as_ref().is_some_and(|s| !s.staged) {
                        p.segment = None;
                    }
                }
                self.user_ring_arm(&record, &mut r);
                Some(false)
            }
            Err(Failure::Refused(why)) => {
                self.poisoned.fetch_add(1, Ordering::Relaxed);
                eprintln!(
                    "kf3: USER_RING_REFUSED {:#x}:{:#x} host {ht:#x}: {why}; no completion/reuse credit",
                    r.key.0, r.key.1
                );
                r.dead = Some(why);
                record.cancel();
                Some(false)
            }
        }
    }

    /// ★ 2026-10-14 (`kf_chan::user_loop`): arm what the turn ended needing. Progress rings the
    /// token again (bounded by `REACT_ROUNDS`, so the worker stays fair); a wait is recorded for the
    /// worker's spin / event / safety-net looks ([`ChanPlane::user_ring_peek`]); nothing else.
    /// No tick, no clock: the relay is rung by its doorbell, by an event, or by a look.
    fn user_ring_arm(&self, record: &userrelay::Cell, r: &mut userrelay::Record) {
        use kf_chan::user_loop::{Need, wait_of};
        let idx = r.idx;
        let Some(p) = r.pump.as_mut() else {
            return;
        };
        let need = p.need;
        p.wait = wait_of(need);
        record.set_waiting(p.wait.is_some());
        match need {
            Need::Again => {
                record.contended_clear();
                if self.plane.ring_internal(idx) {
                    let _ = self.wake.signal();
                }
            }
            Need::Contended => {
                record.contended_note();
                Stats::bump(&self.relay_stats.contended);
            }
            Need::Idle | Need::Marker { .. } | Need::Popped { .. } => record.contended_clear(),
        }
    }

    /// Some relay waits for something only a look can find: a private-ring wait
    /// (`Pump::wait`), or a durable demand (`Cell::retry`) its turn could not act on
    /// (a lock held elsewhere). Cheap: two atomics per relay.
    pub(crate) fn user_ring_waiting(&self) -> bool {
        self.user_rings.try_lock().map_or(true, |rings| {
            rings
                .values()
                .any(|record| record.is_waiting() || record.pending())
        })
    }

    /// ★ 2026-10-14 -- the worker's LOOK at the relays (`kf_chan::worker::RelayLoop::peek`): read each
    /// waiting relay's private `GP_GET` and ring the token of those whose wait is satisfied (or whose
    /// lock was busy). Never a wait, never a scan, never a publication: it only rings. `why` names the
    /// caller; a BACKSTOP look that finds a satisfied wait an interrupt should have announced is the
    /// bug counter, a satisfied control-witness gate / contended relay is counted apart, and `GP_PUT`
    /// ahead of the cursor with no doorbell is information (the contract allows it between a guest's
    /// `GP_PUT` write and its ring).
    pub(crate) fn user_ring_peek(&self, why: kf_chan::user_loop::Peek) -> bool {
        // The map's holders (birth / free) keep it for a handful of instructions and never wait on the
        // worker; the look only reads atomics, `try_lock`s the records and rings.
        let Ok(rings) = self.user_rings.lock() else {
            return false;
        };
        let mut rang = false;
        for record in rings.values() {
            rang |= self.user_ring_peek_one(record, why);
        }
        rang
    }

    fn user_ring_peek_one(
        &self,
        record: &Arc<userrelay::Cell>,
        why: kf_chan::user_loop::Peek,
    ) -> bool {
        use kf_chan::user_loop::{Found, Peek, found};
        if record.is_canceled() {
            return false;
        }
        let backstop = why == Peek::Backstop;
        let demand = record.pending();
        if !demand && !record.is_waiting() && !backstop {
            return false;
        }
        // a relay that keeps finding a busy lock is re-rung by the spin / event looks only a bounded
        // number of times in a row (else the spin is an endless busy loop); the safety net always may
        let retry_ok = kf_chan::user_loop::may_retry(record.contended_run(), why);
        let ring = |this: &Self| {
            if this.plane.ring_internal(record.idx) {
                let _ = this.wake.signal();
            }
            true
        };
        let Ok(mut r) = record.try_lock() else {
            // Held elsewhere (the act thread): ring, the turn handles the contention itself.
            return retry_ok && ring(self);
        };
        if r.retired || r.free_attempted || r.dead.is_some() {
            return false;
        }
        let r = &mut *r;
        let Some(p) = r.pump.as_mut() else {
            return false;
        };
        if let Some(w) = p.wait {
            // The private words: `GP_GET` (entries popped) and the tagged `REFERENCE` (markers
            // processed) are the proofs; the interrupt is only the hint.
            let mut pops = false;
            let satisfied = match r.lease.storage.try_lock() {
                Ok(storage) => match (storage.gpu_get(), storage.gpu_reference()) {
                    (Ok(get), Ok(reference)) => match p.fetch.peek(get) {
                        Ok(popped) => {
                            // pops the guest's `GP_GET` write-back has not shown yet (they carry no event)
                            pops = popped > p.fetch.fetched();
                            w.satisfied(p.marks.peek(reference), popped)
                        }
                        // outside the ring / beyond what was issued: the turn refuses it by name
                        Err(_) => true,
                    },
                    _ => true,
                },
                Err(_) => true,
            };
            if !satisfied && pops && matches!(w, kf_chan::user_loop::Wait::Marker { .. }) {
                // A gate (or batch) is still draining, but the PBDMA has popped more entries than the
                // guest's `GP_GET` shows. The wait stays; the turn only writes the guest's `GP_GET` back
                // (a guest blocked on a full ring behind its OWN gate would otherwise wait for the
                // marker, which waits for the guest).
                Stats::bump(&self.relay_stats.pop_writebacks);
                return ring(self);
            }
            if satisfied && (retry_ok || !matches!(w, kf_chan::user_loop::Wait::Contended)) {
                if backstop {
                    let (counter, name) = match found(w) {
                        Found::Work => {
                            (&self.relay_stats.backstop_found_work, "BACKSTOP_FOUND_WORK")
                        }
                        Found::Carried => (&self.relay_stats.backstop_carried, "BACKSTOP_CARRIED"),
                        Found::Contended => {
                            (&self.relay_stats.backstop_contended, "BACKSTOP_CONTENDED")
                        }
                    };
                    let n = counter.fetch_add(1, Ordering::Relaxed) + 1;
                    if matches!(found(w), Found::Work) && (n <= 8 || n.is_power_of_two()) {
                        eprintln!(
                            "kf3: USER_RING_{name} {:#x}:{:#x} #{n}: the safety net found a satisfied wait {w:?} (an interrupt should have announced it)",
                            r.key.0, r.key.1
                        );
                    }
                }
                p.wait = None;
                record.set_waiting(false);
                return ring(self);
            }
            if why == Peek::Event {
                Stats::bump(&self.relay_stats.event_without_progress);
            }
        } else if backstop && p.segment.is_none() {
            // Information only: GP_PUT ahead of the cursor with no doorbell.
            let mut put = [0u8; 4];
            if r.userd
                .read(
                    self,
                    r.key,
                    &r.allocation,
                    kf_abi::submit::USERD_GP_PUT,
                    &mut put,
                )
                .is_ok()
                && u32::from_le_bytes(put) != p.cursor
            {
                Stats::bump(&self.relay_stats.backstop_put_ahead);
                return ring(self);
            }
        }
        if demand && retry_ok {
            if backstop {
                Stats::bump(&self.relay_stats.backstop_contended);
            }
            return ring(self);
        }
        false
    }

    /// Selects the worker's short park interval ONLY for the measurement trace's deferred re-reads
    /// (`KF3_USER_RING_TRACE`); production never calls a clock-driven turn.
    pub(crate) fn user_ring_poll_due(&self) -> bool {
        tracing()
            && self.user_rings.try_lock().is_ok_and(|rings| {
                rings.values().any(|record| {
                    record.try_lock().is_ok_and(|r| {
                        r.pump
                            .as_ref()
                            .and_then(|p| p.trace.as_ref())
                            .is_some_and(|t| !t.rereads.is_empty())
                    })
                })
            })
    }

    /// Worker tick (`TICK_TAG`): MEASUREMENT ONLY. The trace's deferred re-reads and its idle-stall
    /// report run here; with the trace off this does nothing, and in production no relay is ever
    /// rung by a tick.
    pub(super) fn user_ring_refresh(&self) {
        if !tracing() {
            return;
        }
        let Ok(rings) = self.user_rings.try_lock() else {
            return;
        };
        for record in rings.values() {
            let mut signaled = false;
            record.wake_if(
                |r| {
                    !r.retired
                        && !r.free_attempted
                        && r.dead.is_none()
                        && r.pump
                            .as_ref()
                            .and_then(|p| p.trace.as_ref())
                            .is_some_and(|t| !t.rereads.is_empty())
                },
                |idx| signaled = self.plane.ring_internal(idx),
            );
            if signaled {
                let _ = self.wake.signal();
            }
            if let Ok(mut r) = record.try_lock() {
                self.trace_idle(&mut r);
            }
        }
    }

    /// Measurement only: a pump that has had no turn for 1.5 s while the guest's PUT names
    /// entries it has not read (no doorbell, no tick) — reported once per episode.
    fn trace_idle(&self, r: &mut userrelay::Record) {
        if r.retired || r.free_attempted || r.dead.is_some() {
            return;
        }
        let Some(p) = r.pump.as_ref() else {
            return;
        };
        let Some(t) = p.trace.as_ref() else {
            return;
        };
        if t.idle_reported || t.last_turn.elapsed() < std::time::Duration::from_millis(1500) {
            return;
        }
        let mut b = [0u8; 4];
        let put = r
            .userd
            .read(
                self,
                r.key,
                &r.allocation,
                kf_abi::submit::USERD_GP_PUT,
                &mut b,
            )
            .ok()
            .map(|()| u32::from_le_bytes(b));
        let cursor = p.cursor;
        if put.is_some_and(|put| put != cursor) {
            eprintln!(
                "kf3: USER_RING_TRACE IDLE_STALL {:#x}:{:#x} no turn for {} ms while guest PUT={put:?} != cursor={cursor} (wait={:?} pending={})",
                r.key.0,
                r.key.1,
                t.last_turn.elapsed().as_millis(),
                p.wait,
                false
            );
            if let Some(t) = r.pump.as_mut().and_then(|p| p.trace.as_mut()) {
                t.idle_reported = true;
            }
        }
    }

    pub(super) fn user_ring_wake(&self, token: u32) {
        if let Some(record) = self.user_ring_record(token) {
            if !record.request() {
                return;
            }
            record.wake_if(
                |r| !r.retired && !r.free_attempted && r.dead.is_none(),
                |idx| {
                    self.plane.ring_internal(idx);
                },
            );
            // Even when metadata is busy, wake the worker to observe durable
            // demand; never publish a stale guest idx without its guard.
            let _ = self.wake.signal();
        }
    }

    fn user_ring_ctx_act(
        &self,
        plan: crate::defapi::Planned,
        cell: SwCell,
        ring: u32,
        key: (u32, u32),
        allocation: Arc<()>,
    ) -> Result<(), String> {
        let act: Act = Box::new(move |me| {
            let current = me
                .pt
                .lock()
                .ok()
                .and_then(|pt| {
                    pt.get(&key)
                        .map(|t| t.user_ring && Arc::ptr_eq(&t.allocation, &allocation))
                })
                .unwrap_or(false);
            let result = if current {
                me.deferred_ctx(&plan)
            } else {
                Err((
                    NV_ERR_INVALID_STATE,
                    "USER_RING_CONTEXT_ALLOCATION_GONE".into(),
                ))
            };
            if let Ok(mut cell) = cell.lock() {
                *cell = Some(
                    result
                        .clone()
                        .map_err(|(status, why)| format!("{why} ({status:#x})")),
                );
            }
            if me.plane.ring_internal(ring) {
                let _ = me.wake.signal();
            }
            result
        });
        let sent = self.acts.lock().ok().and_then(|acts| {
            acts.as_ref().map(|tx| {
                tx.send((act, kf_gsp::Deferred::new(), "USER ring context"))
                    .is_ok()
            })
        });
        if sent == Some(true) {
            Ok(())
        } else {
            Err("USER_RING_CONTEXT_ACT_UNAVAILABLE".into())
        }
    }
}

/// ★ 2026-10-14: the worker's hooks for the event loop (`kf_chan::worker::run_with_relay`).
impl kf_chan::worker::RelayLoop for ChanPlane {
    fn waiting(&self) -> bool {
        self.user_ring_waiting()
    }
    fn peek(&self, why: kf_chan::user_loop::Peek) -> bool {
        self.user_ring_peek(why)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_abi::submit::method_header_inc as h;
    fn sw(guest: u32, number: u16) -> Software {
        Software {
            guest,
            number,
            live: Arc::new(AtomicBool::new(true)),
            ready: Arc::new(AtomicBool::new(true)),
        }
    }
    #[test]
    fn classification_is_pinned_across_free_reuse_and_uses_admitted_class_id() {
        let software = Some(sw(0x51, 7));
        let words = [h(5, 0, 1).unwrap(), 7, h(5, 0x200, 1).unwrap(), 29, 0];
        let scan = Scan::default();
        let (patched, rare) = stage_plan(&scan, &software, &words).unwrap();
        assert!(rare);
        assert_eq!(patched, [words[0], 7, 0, 0, 0]);
        let old = software.as_ref().unwrap();
        old.live.store(false, Ordering::Release);
        let replacement = Some(sw(0x51, 7));
        assert!(!same_software(&software, &replacement));
        assert!(!same_software(&software, &None));
        // Same numeric handles cannot alter this already prepared plan.
        let mut prefix = scan.clone();
        let trigger = prefix.segment(&words, |v| old.matches(v)).unwrap().unwrap();
        assert_eq!((trigger.start, trigger.end, trigger.call.data), (2, 4, 29));
        assert_eq!(
            stage_plan(&scan, &software, &words).unwrap(),
            (patched, true)
        );
        assert!(!old.matches(1));
        assert!(old.matches(crate::defapi::NV50_DEFERRED_API_CLASS));
    }
    /// ★ 2026-10-14: the measured migration trigger (Edge, run 2990 at f163b85e: the last 8 words of a
    /// 56-word segment) sits inside an FE atomic sequence on the compute subchannel. The gapless remainder
    /// is the whole segment: BEGIN, WFI, the trigger as two DMA_NOPs, END — no split between BEGIN and END.
    /// Before this, the staged prefix ended at the trigger and END followed only after the action.
    #[test]
    fn a_channel_update_trigger_inside_an_atomic_sequence_is_published_without_a_gap() {
        let software = Some(sw(0x51, 5));
        let words = [
            0x0000_0286,
            0x8000_a01e,
            0x8000_a009,
            0x8000_2052,
            0x8001_201e,
            0x2001_a080,
            0x4000_00a1,
            0x8000_2053,
        ];
        let words: Vec<u32> = std::iter::once(h(5, 0, 1).unwrap())
            .chain(std::iter::once(5))
            .chain(words.iter().skip(1).copied())
            .collect();
        let scan = Scan::for_subdevice(1).unwrap();
        let matches = |v: u32| software.as_ref().unwrap().matches(v);
        let (patched, rare) = stage_plan(&scan, &software, &words).unwrap();
        assert!(rare);
        let mut next = scan.clone();
        let t = next.segment(&words, matches).unwrap().unwrap();
        assert_eq!((t.start, t.end), (6, 8), "the trigger packet");
        // Before: the staged prefix ends at the trigger, END (index 8) only after the action.
        assert_eq!(&patched[..t.start], &words[..6]);
        let (end, after) = gapless_remainder(&next, &words, 0, t, matches).expect("gapless");
        assert_eq!(end, words.len());
        assert!(after.at_boundary());
        // One staged copy: BEGIN (0x148 on sub1) … two DMA_NOPs … END (0x14c on sub1), contiguous.
        assert_eq!(
            &patched[..end],
            &[&words[..6], &[0, 0][..], &words[8..]].concat()[..]
        );
        assert!(patched[..end].contains(&0x8000_2052) && patched[..end].contains(&0x8000_2053));
        // A second trigger in the remainder keeps the split.
        let mut two = words.clone();
        two.extend([h(5, 0x200, 1).unwrap(), 7]);
        let mut n2 = scan.clone();
        let t2 = n2.segment(&two, matches).unwrap().unwrap();
        assert!(gapless_remainder(&n2, &two, 0, t2, matches).is_none());
    }

    #[test]
    fn masked_methods_stay_native_and_only_active_triggers_are_staged() {
        let software = Some(sw(0x51, 1));
        let scan = Scan::for_subdevice(1).unwrap();
        let words = [
            h(6, 0, 1).unwrap(),
            1,
            0x10000,
            h(6, 0x200, 1).unwrap(),
            27,
            0x10010,
            h(6, 0x200, 1).unwrap(),
            28,
            0x10000,
        ];
        let (patched, rare) = stage_plan(&scan, &software, &words).unwrap();
        assert!(rare);
        let mut expected = words;
        expected[6..8].fill(0);
        assert_eq!(patched, expected);
        let mut submitted = scan.clone();
        let trigger = submitted
            .segment(&words, |v| software.as_ref().unwrap().matches(v))
            .unwrap()
            .unwrap();
        assert_eq!((trigger.start, trigger.end, trigger.call.data), (6, 8, 28));
        assert!(
            submitted
                .segment(&words[trigger.end..], |v| software
                    .as_ref()
                    .unwrap()
                    .matches(v))
                .unwrap()
                .is_none()
        );
        // Final inactive state persists; a later stream restores it explicitly.
        assert!(
            submitted
                .segment(&[h(6, 0x200, 1).unwrap(), 29], |v| software
                    .as_ref()
                    .unwrap()
                    .matches(v))
                .unwrap()
                .is_none()
        );
        assert_eq!(scan, Scan::for_subdevice(1).unwrap());
    }
    #[test]
    fn all_rare_packets_are_nopped_but_the_prefix_scan_stops_at_each_exact_trigger() {
        let software = Some(sw(0x51, 1));
        let words = [
            h(5, 0, 1).unwrap(),
            1,
            h(5, 0x200, 1).unwrap(),
            27,
            0,
            h(5, 0x200, 1).unwrap(),
            28,
            0,
        ];
        let scan = Scan::default();
        let (patched, rare) = stage_plan(&scan, &software, &words).unwrap();
        assert!(rare);
        assert_eq!(patched, [words[0], 1, 0, 0, 0, 0, 0, 0]);
        let mut state = scan.clone();
        let a = state
            .segment(&words, |v| software.as_ref().unwrap().matches(v))
            .unwrap()
            .unwrap();
        let b = state
            .segment(&words[a.end..], |v| software.as_ref().unwrap().matches(v))
            .unwrap()
            .unwrap();
        assert_eq!((a.start, a.end, b.start, b.end), (2, 4, 1, 3));
        assert_eq!(scan, Scan::default()); // planning never advances submitted state
    }
    #[test]
    fn a_rare_fragmented_suffix_refuses_without_advancing_the_scan() {
        let software = Some(sw(0x51, 1));
        let words = [
            h(5, 0, 1).unwrap(),
            1,
            h(5, 0x200, 1).unwrap(),
            27,
            h(2, 0x300, 2).unwrap(),
            8,
        ];
        let scan = Scan::default();
        assert_eq!(
            stage_plan(&scan, &software, &words),
            Err("USER_RING_TRIGGER_PARTIAL_SUFFIX_UNSUPPORTED".into())
        );
        assert_eq!(scan, Scan::default());
    }
    #[test]
    fn descriptor_fetch_reports_old_source_before_evict_or_migration_and_keeps_staging() {
        let mut fetch = Fetch::new(8).unwrap();
        fetch.publish(source_receipt(false, true, 1, 17)).unwrap();
        fetch.publish(None).unwrap();
        fetch.publish(None).unwrap();
        let witness = fetch.issued();
        assert_eq!(fetch.observe(1).unwrap(), [(1, 17)]);
        assert!(fetch.fetched() < witness); // old source retirement is NOT PB reuse
        assert_eq!(source_receipt(true, true, 1, 17), None);
        fetch.observe(3).unwrap();
        assert_eq!(fetch.fetched(), witness);
        // Migration changes only source generation; private native GET continues.
        fetch.publish(source_receipt(false, true, 2, 1)).unwrap();
        assert_eq!(fetch.observe(4).unwrap(), [(2, 1)]);
    }
    #[test]
    fn evict_at_end_finishes_after_prefix_receipt_without_a_new_native_suffix() {
        let software = Some(sw(0x51, 1));
        let words = [h(5, 0, 1).unwrap(), 1, h(5, 0x200, 1).unwrap(), 27];
        let mut scan = Scan::default();
        let trigger = scan
            .segment(&words, |v| software.as_ref().unwrap().matches(v))
            .unwrap()
            .unwrap();
        assert_eq!(
            suffix_after_trigger(0, trigger, words.len()),
            Ok((words.len(), true))
        );
        let mut fetch = Fetch::new(8).unwrap();
        let (first, _) = fetch.publish(source_receipt(false, true, 1, 1)).unwrap();
        assert_eq!(first, 0);
        fetch.publish(None).unwrap();
        fetch.publish(None).unwrap();
        fetch.observe(3).unwrap();
        let native_issued = fetch.issued();
        // Completing the CPU action adds no GPU work and needs no post-evict GET.
        assert!(suffix_after_trigger(0, trigger, words.len()).unwrap().1);
        assert_eq!(fetch.issued(), native_issued);
    }
    #[test]
    fn contention_retries_but_poison_revocation_and_uncertain_io_do_not() {
        let lock = Mutex::new(());
        let _guard = lock.lock().unwrap();
        assert!(matches!(
            locked(lock.try_lock(), "POISON"),
            Err(Failure::Retry)
        ));
        assert_eq!(
            source_error(userread::Refusal::Busy, "SOURCE"),
            Failure::Retry
        );
        assert_eq!(
            source_error(userread::Refusal::Changed, "SOURCE"),
            Failure::Retry
        );
        assert_eq!(
            access_error(kf_rm::memory_list::AccessError::Busy, "USERD"),
            Failure::Retry
        );
        assert!(matches!(
            source_error(userread::Refusal::Topology, "SOURCE"),
            Failure::Refused(_)
        ));
        assert!(matches!(
            access_error(kf_rm::memory_list::AccessError::Gone, "USERD"),
            Failure::Refused(_)
        ));
        assert!(matches!(
            Failure::from("NATIVE_DOORBELL_UNCERTAIN"),
            Failure::Refused(_)
        ));
    }
}
