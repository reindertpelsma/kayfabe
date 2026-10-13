//! Worker-only restricted USER ring relay. No vCPU/drainer caller may run a
//! turn: source-view preparation can call RM. Native USER privileges and own
//! mirrored mappings enforce access; scanning is never a completion authority.

use super::*;
use crate::mem::userread::{self, Mapping, Snapshot};
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
}
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
    pub tick: bool,
    again: bool,
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
    wait: Option<Wait>,
}
struct Wait {
    witness: u64,
    trigger: Option<Trigger>,
    action: Option<PendingAction>,
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
            tick: false,
            again: false,
            trace: trace_dir().map(|_| Box::new(Trace::new())),
        })
    }
    pub(super) fn release(&mut self) -> Result<(), String> {
        if let Some(t) = &self.trace {
            eprintln!(
                "kf3: USER_RING_TRACE SUMMARY {:x?} segments={} puts_with_doorbell={} puts_without_doorbell={} straddles={} verified={} stale={} stale_desync={}",
                self.key,
                t.seq,
                t.puts_with_db,
                t.puts_without_db,
                t.straddles,
                t.verified,
                t.stale,
                t.stale_desync
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
            let fresh: Option<Vec<u32>> = self.snapshot(me, r, x.va, x.bytes).ok().map(|s| {
                s.bytes
                    .chunks_exact(4)
                    .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
                    .collect()
            });
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
                if !diffs.is_empty() {
                    t.stale += 1;
                    let desync = *end_at_scan != end;
                    if desync {
                        t.stale_desync += 1;
                    }
                    if t.stale <= 64 || desync && t.stale_desync <= 64 {
                        eprintln!(
                            "kf3: USER_RING_TRACE STALE {:#x}:{:#x} seq={} after_us={} words={} changed={} first_diff={:?} last_diff={:?} carried_out_at_scan={end_at_scan} now={end} verified={} stale={} stale_desync={}",
                            r.key.0,
                            r.key.1,
                            x.seq,
                            now.duration_since(*scanned_at).as_micros(),
                            x.words.len(),
                            diffs.len(),
                            diffs.first(),
                            diffs.last(),
                            t.verified,
                            t.stale,
                            t.stale_desync
                        );
                    }
                }
                if t.verified % 5000 == 0 {
                    eprintln!(
                        "kf3: USER_RING_TRACE VERIFY {:#x}:{:#x} verified={} stale={} stale_desync={}",
                        r.key.0, r.key.1, t.verified, t.stale, t.stale_desync
                    );
                }
                continue;
            }
            eprintln!(
                "kf3: USER_RING_TRACE STRADDLE_REREAD {:#x}:{:#x} seq={} +{}ms changed={} first_diff={:?} carried_out_now={end}",
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
            let words2: Option<Vec<u32>> = self
                .snapshot(me, r, d.gpu_va, d.len_bytes as usize)
                .ok()
                .map(|s| {
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
                "reread+{delay}ms entry_same={:?} put={put2:?} doorbells={} words_changed={changed:?} first_diff={first:?} rescan={rescan:?}",
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
            "kf3: USER_RING_TRACE {:#x}:{:#x} cursor={} put={put} entry={entry:#018x} pb_va={:#x} bytes={} db_at_put={db_at} db_prev_put={db_prev} puts_with_db={pw} puts_without_db={pwo} {} file={name} written={}",
            r.key.0,
            r.key.1,
            self.cursor,
            d.gpu_va,
            d.len_bytes,
            summary.join(" "),
            written.is_ok()
        );
    }

    fn turn(&mut self, me: &ChanPlane, r: &mut userrelay::Record) -> Turn<bool> {
        self.tick = false;
        self.again = false;
        if self.trace.as_ref().is_some_and(|t| !t.rereads.is_empty()) {
            let software = software_snapshot(me, r).ok().flatten();
            self.trace_due(me, r, &software);
            // Keep the worker ticking until the deferred re-reads have run.
            self.tick = true;
        }
        let native = r.chan.ok_or("USER_RING_NATIVE_ABSENT")?;
        let get = r
            .lease
            .storage
            .lock()
            .map_err(|_| "USER_RING_STORAGE_POISONED")?
            .gpu_get()?;
        for (generation, get) in self.fetch.observe(get).map_err(str::to_owned)? {
            if generation == self.generation {
                self.guest_get = Some((generation, get));
            }
        }
        if let Some((generation, get)) = self.guest_get {
            if generation == self.generation {
                // Logical memory access revalidates Free/reuse and RAM topology.
                match r.userd.write(
                    me,
                    r.key,
                    &r.allocation,
                    kf_abi::submit::USERD_GP_GET,
                    &get.to_le_bytes(),
                ) {
                    Ok(()) => self.guest_get = None,
                    Err(kf_rm::memory_list::AccessError::Busy) => {
                        self.tick = true;
                        return Ok(false);
                    }
                    Err(e) => return Err(format!("USER_RING_GET_WRITE {e:?}").into()),
                }
            } else {
                self.guest_get = None;
            }
        }
        self.tick = self.fetch.fetched() != self.fetch.issued()
            || self.trace.as_ref().is_some_and(|t| !t.rereads.is_empty());
        if self.segment.is_none() {
            if me.inbox.walk_busy() {
                me.inbox.wait_walk(r.idx);
                return Ok(false);
            }
            // Doorbells counted BEFORE the PUT read (measurement only today).
            let doorbells = me.queued(r.idx);
            let mut put = [0u8; 4];
            r.userd
                .read(
                    me,
                    r.key,
                    &r.allocation,
                    kf_abi::submit::USERD_GP_PUT,
                    &mut put,
                )
                .map_err(|e| access_error(e, "USER_RING_PUT_READ"))?;
            let put = u32::from_le_bytes(put);
            if put >= r.source.1 {
                return Err("USER_RING_PUT_BOUND".into());
            }
            if put == self.cursor {
                return Ok(false);
            }
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
            let source = self.snapshot(me, r, va, 8)?;
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
            if matches!(descriptor, kf_chan::user_stream::Source::Nop) {
                if self.fetch.available() == 0 {
                    self.tick = true;
                    return Ok(false);
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
                self.source_admitted = true;
                self.cursor = guest_get;
                self.tick = true;
                self.again = true;
                return Ok(true);
            }
            let kf_chan::user_stream::Source::Segment(descriptor) = descriptor else {
                return Err("USER_RING_GP_FORMAT".into());
            };
            let pb = self.snapshot(
                me,
                r,
                descriptor.gpu_va,
                usize::try_from(descriptor.len_bytes).map_err(|_| "USER_RING_PB_SIZE")?,
            )?;
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
            let planned = stage_plan(&self.scan, &software, &words);
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
                wait: None,
            });
        }
        let mut segment = self.segment.take().ok_or("USER_RING_SEGMENT_ABSENT")?;
        let outcome = self.segment_turn(me, r, native, &mut segment);
        match outcome {
            Ok((submitted, finished)) => {
                if !finished {
                    self.segment = Some(segment);
                    self.tick = true;
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
            self.tick = true;
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
        self.tick = true;
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
        me.rm
            .doorbell(r.chan.ok_or("USER_RING_NATIVE_ABSENT")?.token)
            .map_err(|e| format!("USER_RING_DOORBELL_UNCERTAIN {e:?}").into())
    }

    fn segment_turn(
        &mut self,
        me: &ChanPlane,
        r: &mut userrelay::Record,
        _native: kf_host::Channel,
        s: &mut Segment,
    ) -> Turn<(bool, bool)> {
        if let Some(mut wait) = s.wait.take() {
            if self.fetch.fetched() < wait.witness {
                s.wait = Some(wait);
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
            if self.fetch.available() == 0 {
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
            self.source_admitted = true;
            self.scan = next;
            self.cursor = s.guest_get;
            self.tick = true;
            return Ok((true, true));
        }
        let end = trigger.map_or(s.words.len(), |t| s.at.saturating_add(t.start));
        let bytes = end
            .checked_sub(s.at)
            .and_then(|n| n.checked_mul(4))
            .ok_or("USER_RING_PREFIX_BOUND")?;
        let credits = if bytes == 0 { 2 } else { 3 };
        if self.fetch.available() < credits {
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
                storage.write_host_witness(witness)?;
                storage.publish_put(put)
            },
        )?;
        if !s.staged && s.generation == self.generation {
            self.source_admitted = true;
            self.cursor = s.guest_get;
        }
        s.staged = true;
        self.scan = next;
        s.wait = Some(Wait {
            witness: self.fetch.issued(),
            trigger,
            action: None,
        });
        Ok((true, false))
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
                    self.guest_get = None;
                })
                .map_err(|e| source_error(e, "USER_RING_MIGRATION_ADMISSION"))
            })
            .map_err(|e| access_error(e, "USER_RING_MIGRATION_USERD_ADMISSION"))??;
        eprintln!(
            "kf3: USER_RING_MIGRATED {:#x}:{:#x} source={:#x}x{} generation={generation}; private native ring/USERD unchanged; no host migration verb",
            r.key.0, r.key.1, u.gpfifo_va, u.entries
        );
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
        let record = self.user_ring_record(ht)?;
        let Ok(mut r) = record.enter() else {
            return Some(false);
        };
        if r.retired || r.free_attempted || r.dead.is_some() {
            record.cancel();
            return Some(false);
        }
        let pt = match self.pt.try_lock() {
            Ok(pt) => pt,
            Err(std::sync::TryLockError::WouldBlock) => return Some(false),
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
            return Some(false);
        }
        if self.stop.load(Ordering::Acquire) {
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
        match outcome {
            Ok(submitted) => {
                // Drain a coalesced source delta without waiting for the park
                // tick. One extra turn may observe a pending native receipt;
                // that turn then parks rather than spinning on the GPU.
                if r.pump.as_ref().is_some_and(|p| p.again) && self.plane.ring_internal(r.idx) {
                    let _ = self.wake.signal();
                }
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
                    p.tick = true;
                }
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

    /// Only selects the existing worker's short park interval. No timeout ever
    /// grants a receipt, retires bytes, or releases a stalled action.
    pub(crate) fn user_ring_poll_due(&self) -> bool {
        self.user_rings.try_lock().is_ok_and(|rings| {
            rings.values().any(|record| {
                !record.is_canceled()
                    && (record.pending()
                        || record.try_lock().is_ok_and(|r| {
                            !r.retired
                                && !r.free_attempted
                                && r.dead.is_none()
                                && r.pump.as_ref().is_some_and(|p| p.tick)
                        }))
            })
        })
    }

    /// Worker tick may queue tokens only, never prepare views or scan.
    pub(super) fn user_ring_refresh(&self) {
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
                        && (record.pending() || r.pump.as_ref().is_some_and(|p| p.tick))
                },
                |idx| signaled = self.plane.ring_internal(idx),
            );
            if signaled {
                let _ = self.wake.signal();
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
