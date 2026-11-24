//! (`docs/design/V3_VA_PROFILE.md`): **bounded phase profiling of the VA apply path**
//! — OBSERVATION ONLY. Nothing here decides anything; the VA manager's acknowledgements, commits,
//! capacity limits and ordering are unchanged by it (`vasmgr::tests::profiling_*`).
//!
//! - [`Log2Hist`]: a fixed log2 histogram of microseconds (32 buckets, no allocation), with
//!   percentiles reported as the bucket's exclusive upper bound.
//! - [`SlowRecord`] / [`SlowRing`]: a fixed ring of the slowest tickets and applies (the newest
//!   [`SLOW_RING`] kept, the overwritten ones counted).
//! - [`DumpBudget`]: a token bucket over a caller's clock, so the detailed lines of a relaunch late
//!   in a boot are printed too, never more than [`DUMP_CAP`] per boot.
//! - [`vat_old_fields`] / [`vat_new_fields`]: the `vat[...]` block of the status line. The old
//!   fields keep their names, order and meaning (the extraction `vat\[(.*?)\]` then `(\w+)=(\d+)`
//!   keeps working); the new ones are appended after them.

use kf_host::phase::{Acc, COUNT, Phase, Phases};

/// Buckets of [`Log2Hist`].
pub const HIST_BUCKETS: usize = 32;

/// ★ A detailed record is kept for a ticket whose request latency, or a space whose apply, is longer
/// than this (ns).
pub const SLOW_PROFILE_NS: u64 = 250_000_000;

/// Records the ring keeps.
pub const SLOW_RING: usize = 64;

/// Detailed lines printed per boot at most (the ring summary at the end is extra).
pub const DUMP_CAP: u64 = 256;

/// Lines that may be printed back to back.
pub const DUMP_BURST: u32 = 16;

/// One more line may be printed every this many ns.
pub const DUMP_REFILL_NS: u64 = 4_000_000_000;

/// A log2 histogram of durations in microseconds: bucket 0 is `[0, 2)` us, bucket `i >= 1` is
/// `[2^i, 2^(i+1))` us, bucket 31 is everything from `2^31` us on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Log2Hist {
    /// Samples per bucket.
    pub b: [u64; HIST_BUCKETS],
}

impl Default for Log2Hist {
    fn default() -> Self {
        Log2Hist {
            b: [0; HIST_BUCKETS],
        }
    }
}

/// The bucket of a duration of `ns`.
#[must_use]
pub fn bucket_of_ns(ns: u64) -> usize {
    let us = ns / 1000;
    if us < 2 {
        0
    } else {
        63usize
            .saturating_sub(us.leading_zeros() as usize)
            .min(HIST_BUCKETS - 1)
    }
}

/// The exclusive upper bound of bucket `i`, in us (the last bucket reports `2^32`).
#[must_use]
pub fn bucket_upper_us(i: usize) -> u64 {
    let shift = u32::try_from(i.min(HIST_BUCKETS - 1)).unwrap_or(31);
    1u64.checked_shl(shift.saturating_add(1))
        .unwrap_or(u64::MAX)
}

impl Log2Hist {
    /// Count one sample of `ns`.
    pub fn record_ns(&mut self, ns: u64) {
        if let Some(x) = self.b.get_mut(bucket_of_ns(ns)) {
            *x = x.saturating_add(1);
        }
    }

    /// Samples counted.
    #[must_use]
    pub fn count(&self) -> u64 {
        self.b.iter().fold(0u64, |a, &x| a.saturating_add(x))
    }

    /// The samples counted since `prev` (bucket by bucket).
    #[must_use]
    pub fn since(&self, prev: &Log2Hist) -> Log2Hist {
        let mut out = *self;
        for (a, b) in out.b.iter_mut().zip(prev.b.iter()) {
            *a = a.saturating_sub(*b);
        }
        out
    }

    /// The `permille`-th percentile as the exclusive upper bound (us) of the bucket holding the
    /// sample of rank `ceil(count * permille / 1000)`; `0` when empty. "p95_us=X": 95 % of the
    /// samples were shorter than X us.
    #[must_use]
    pub fn percentile_us(&self, permille: u64) -> u64 {
        let n = self.count();
        if n == 0 {
            return 0;
        }
        let rank = n.saturating_mul(permille.min(1000)).div_ceil(1000).max(1);
        let mut cum = 0u64;
        for (i, &c) in self.b.iter().enumerate() {
            cum = cum.saturating_add(c);
            if cum >= rank {
                return bucket_upper_us(i);
            }
        }
        bucket_upper_us(HIST_BUCKETS - 1)
    }
}

/// What a [`SlowRecord`] describes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SlowKind {
    /// One space's apply.
    #[default]
    Apply,
    /// One split ticket, from its enqueue (or acceptance) to its verdict.
    Ticket,
}

/// ★ One slow ticket or apply. Every field is a host clock reading or a host-assigned id/count;
/// nothing is guest bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SlowRecord {
    /// Apply or ticket.
    pub kind: SlowKind,
    /// ms since the manager was built (the VA thread's clock).
    pub t_ms: u64,
    /// The split ticket (0: none — an invalidate or a root walk).
    pub ticket: u64,
    /// The walk (`walk#` of the other lines).
    pub walk: u64,
    /// The space key (`VasKey`; 0 for a ticket naming several).
    pub key: u64,
    /// The root (page directory) walked; for a ticket naming one root, that root.
    pub pdb: u64,
    /// The walker slot.
    pub slot: u32,
    /// This window's ordinal for the request (1 = its first walk).
    pub window: u32,
    /// Ticket: windows (walks) it rode in all.
    pub windows: u32,
    /// Ticket: worker enqueue -> manager acceptance, us (0 when the enqueue time is unknown).
    pub q_inbox_us: u64,
    /// Ticket: manager acceptance -> verdict, us.
    pub mgr_us: u64,
    /// Ticket: enqueue (else acceptance) -> verdict, us.
    pub req_us: u64,
    /// Ticket: summed submit wall of its windows, us.
    pub submit_us: u64,
    /// Ticket: summed submit -> collected of its windows (GPU walk, wake, decode, retries), us.
    pub walk_us: u64,
    /// Ticket: summed collect/decode (`Walker::poll` returning the report), us.
    pub collect_us: u64,
    /// Ticket: summed applies of the spaces it names, us.
    pub apply_own_us: u64,
    /// Ticket: summed applies of every space of its windows (its own included), us.
    pub apply_all_us: u64,
    /// Ticket: hidden capacity re-walks inside its windows.
    pub cap_retries: u64,
    /// Apply: wall of this space's apply, us.
    pub apply_us: u64,
    /// Apply: runs mapped.
    pub maps: u64,
    /// Apply: runs unmapped.
    pub unmaps: u64,
    /// Apply: the phases of this apply.
    pub phases: Phases,
}

impl SlowRecord {
    /// One `key=value` line (no brackets; values are integers or hex ids).
    #[must_use]
    pub fn line(&self, seq: u64) -> String {
        let us = |ns: u64| ns / 1000;
        match self.kind {
            SlowKind::Ticket => format!(
                "kf3: VA_SLOW n={seq} kind=ticket t_ms={} ticket={} walk={} key={:#x} pdb={:#x} windows={} req_us={} q_inbox_us={} mgr_us={} submit_us={} walk_us={} collect_us={} apply_own_us={} apply_all_us={} cap_retries={}",
                self.t_ms,
                self.ticket,
                self.walk,
                self.key,
                self.pdb,
                self.windows,
                self.req_us,
                self.q_inbox_us,
                self.mgr_us,
                self.submit_us,
                self.walk_us,
                self.collect_us,
                self.apply_own_us,
                self.apply_all_us,
                self.cap_retries,
            ),
            SlowKind::Apply => {
                let mut s = format!(
                    "kf3: VA_SLOW n={seq} kind=apply t_ms={} ticket={} walk={} key={:#x} pdb={:#x} slot={} window={} apply_us={} maps={} unmaps={}",
                    self.t_ms,
                    self.ticket,
                    self.walk,
                    self.key,
                    self.pdb,
                    self.slot,
                    self.window,
                    self.apply_us,
                    self.maps,
                    self.unmaps,
                );
                for p in Phase::ALL {
                    let a = self.phases.get(p);
                    s.push_str(&format!(
                        " {0}_n={1} {0}_us={2} {0}_max_us={3}",
                        p.name(),
                        a.n,
                        us(a.ns),
                        us(a.max)
                    ));
                }
                s
            }
        }
    }
}

/// ★ A fixed ring of the newest [`SLOW_RING`] slow records. Never grows; an overwritten record is
/// counted ([`SlowRing::overwritten`]).
#[derive(Debug, Clone)]
pub struct SlowRing {
    buf: [SlowRecord; SLOW_RING],
    seq: [u64; SLOW_RING],
    printed: [bool; SLOW_RING],
    head: usize,
    len: usize,
    pushed: u64,
    overwritten: u64,
}

impl Default for SlowRing {
    fn default() -> Self {
        SlowRing {
            buf: [SlowRecord::default(); SLOW_RING],
            seq: [0; SLOW_RING],
            printed: [false; SLOW_RING],
            head: 0,
            len: 0,
            pushed: 0,
            overwritten: 0,
        }
    }
}

impl SlowRing {
    /// Keep `r`; returns its sequence number (1-based, boot-wide). The oldest is overwritten when full.
    pub fn push(&mut self, r: SlowRecord) -> u64 {
        if self.len == SLOW_RING {
            self.overwritten = self.overwritten.saturating_add(1);
        } else {
            self.len = self.len.saturating_add(1);
        }
        self.pushed = self.pushed.saturating_add(1);
        let h = self.head;
        if let (Some(b), Some(q), Some(p)) = (
            self.buf.get_mut(h),
            self.seq.get_mut(h),
            self.printed.get_mut(h),
        ) {
            *b = r;
            *q = self.pushed;
            *p = false;
        }
        self.head = wrap(h.saturating_add(1));
        self.pushed
    }

    /// Mark record `seq` printed (if it is still held).
    pub fn mark_printed(&mut self, seq: u64) {
        if seq == 0 {
            return;
        }
        if let Some(i) = self.seq.iter().position(|&s| s == seq)
            && let Some(p) = self.printed.get_mut(i)
        {
            *p = true;
        }
    }

    /// Records held (at most [`SLOW_RING`]).
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Nothing held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Records ever pushed.
    #[must_use]
    pub fn pushed(&self) -> u64 {
        self.pushed
    }

    /// Records overwritten before anyone read them out.
    #[must_use]
    pub fn overwritten(&self) -> u64 {
        self.overwritten
    }

    /// The held records, oldest first, with their sequence number and whether they were printed.
    #[must_use]
    pub fn held(&self) -> Vec<(u64, SlowRecord, bool)> {
        let start = wrap(self.head.saturating_add(SLOW_RING).saturating_sub(self.len));
        (0..self.len)
            .filter_map(|k| {
                let i = wrap(start.saturating_add(k));
                Some((*self.seq.get(i)?, *self.buf.get(i)?, *self.printed.get(i)?))
            })
            .collect()
    }
}

/// An index one past the ring's end, back to its start (indices stay below `2 * SLOW_RING`).
fn wrap(i: usize) -> usize {
    if i >= SLOW_RING {
        i.saturating_sub(SLOW_RING)
    } else {
        i
    }
}

/// ★ A token bucket over the caller's clock (ns): [`DUMP_BURST`] lines back to back, one more
/// every [`DUMP_REFILL_NS`], never more than [`DUMP_CAP`] per boot. Counts what it refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DumpBudget {
    burst: u32,
    refill_ns: u64,
    cap: u64,
    tokens: u32,
    last_ns: u64,
    printed: u64,
    suppressed: u64,
}

impl Default for DumpBudget {
    fn default() -> Self {
        DumpBudget::new(DUMP_BURST, DUMP_REFILL_NS, DUMP_CAP)
    }
}

impl DumpBudget {
    /// A budget of `burst` lines, one more every `refill_ns`, at most `cap` in all.
    #[must_use]
    pub fn new(burst: u32, refill_ns: u64, cap: u64) -> Self {
        DumpBudget {
            burst,
            refill_ns: refill_ns.max(1),
            cap,
            tokens: burst,
            last_ns: 0,
            printed: 0,
            suppressed: 0,
        }
    }

    /// May one line be printed at `now_ns` (monotonic, the same clock every call)?
    pub fn admit(&mut self, now_ns: u64) -> bool {
        let elapsed = now_ns.saturating_sub(self.last_ns);
        let add = elapsed.checked_div(self.refill_ns).unwrap_or(0);
        if add > 0 {
            let add32 = u32::try_from(add).unwrap_or(u32::MAX);
            self.tokens = self.tokens.saturating_add(add32).min(self.burst);
            self.last_ns = self
                .last_ns
                .saturating_add(add.saturating_mul(self.refill_ns));
        }
        if self.printed < self.cap && self.tokens > 0 {
            self.tokens = self.tokens.saturating_sub(1);
            self.printed = self.printed.saturating_add(1);
            true
        } else {
            self.suppressed = self.suppressed.saturating_add(1);
            false
        }
    }

    /// Lines admitted.
    #[must_use]
    pub fn printed(&self) -> u64 {
        self.printed
    }

    /// Lines refused.
    #[must_use]
    pub fn suppressed(&self) -> u64 {
        self.suppressed
    }
}

/// ★ The VA manager's slow-record sink: the ring, its print budget and their clock origin.
#[derive(Debug, Clone)]
pub struct SlowSink {
    /// The ring.
    pub ring: Box<SlowRing>,
    /// The print budget.
    pub dump: DumpBudget,
    /// The clock origin (`t_ms`, the budget's `now`).
    pub born: std::time::Instant,
}

impl Default for SlowSink {
    fn default() -> Self {
        SlowSink {
            ring: Box::default(),
            dump: DumpBudget::default(),
            born: std::time::Instant::now(),
        }
    }
}

impl SlowSink {
    /// Keep `r`; print it now (stderr, the caller's thread) if the budget admits it; refresh the
    /// boot-wide counters in `pc`.
    pub fn keep(&mut self, mut r: SlowRecord, pc: &mut VaProfCounters) {
        let now = self.born.elapsed();
        r.t_ms = u64::try_from(now.as_millis()).unwrap_or(u64::MAX);
        let seq = self.ring.push(r);
        if self
            .dump
            .admit(u64::try_from(now.as_nanos()).unwrap_or(u64::MAX))
        {
            eprintln!("{}", r.line(seq));
            self.ring.mark_printed(seq);
        }
        pc.slow_printed = self.dump.printed();
        pc.slow_suppressed = self.dump.suppressed();
        pc.ring_overwritten = self.ring.overwritten();
    }
}

/// ★ Per split ticket, carried with the request across its windows (a `Copy` value in the want).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TicketProf {
    /// Worker enqueue -> manager acceptance, ns (`None`: the enqueue time is unknown).
    pub q_inbox_ns: Option<u64>,
    /// Windows (walks) ridden so far.
    pub windows: u32,
    /// Summed submit wall.
    pub submit_ns: u64,
    /// Summed submit -> collected.
    pub walk_ns: u64,
    /// Summed collect/decode.
    pub collect_ns: u64,
    /// Summed applies of the spaces it names.
    pub apply_own_ns: u64,
    /// Summed applies of every space of its windows.
    pub apply_all_ns: u64,
    /// Hidden capacity re-walks inside its windows.
    pub cap_retries: u64,
    /// The last walk it rode.
    pub last_walk: u64,
}

/// ★ The new `vat[...]` counters (cumulative in [`crate::vasmgr::VaTiming`]; the status line prints
/// the window since its last line, maxima boot-wide).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VaProfCounters {
    /// Split tickets with a known enqueue time: enqueue -> acceptance.
    pub q_inbox: Acc,
    /// Walk submits (the manager thread's cost).
    pub submit: Acc,
    /// `Walker::poll` calls that returned a report (decode, validation, conversion).
    pub collect: Acc,
    /// Hidden capacity re-walks (`Walker::capacity_retries` moved).
    pub cap_retries: u64,
    /// Split tickets: enqueue (else acceptance) -> verdict.
    pub req: Acc,
    /// Its histogram.
    pub req_hist: Log2Hist,
    /// Per-space applies.
    pub aps: Acc,
    /// Its histogram.
    pub aps_hist: Log2Hist,
    /// The applies' phases.
    pub phases: Phases,
    /// Tickets over [`SLOW_PROFILE_NS`] (boot-wide).
    pub slow_reqs: u64,
    /// Applies over [`SLOW_PROFILE_NS`] (boot-wide).
    pub slow_aps: u64,
    /// The most windows one ticket rode (boot-wide).
    pub windows_max: u64,
    /// Slow records printed / refused by the dump budget / overwritten in the ring (boot-wide).
    pub slow_printed: u64,
    /// See [`VaProfCounters::slow_printed`].
    pub slow_suppressed: u64,
    /// See [`VaProfCounters::slow_printed`].
    pub ring_overwritten: u64,
}

impl VaProfCounters {
    /// The window since `prev`: counts and sums subtract, histograms bucket by bucket; maxima and
    /// the boot-wide `slow_*`, `windows_max`, `ring_overwritten` are `self`'s.
    #[must_use]
    pub fn since(&self, prev: &VaProfCounters) -> VaProfCounters {
        let d = |a: Acc, b: Acc| Acc {
            n: a.n.saturating_sub(b.n),
            ns: a.ns.saturating_sub(b.ns),
            max: a.max,
        };
        VaProfCounters {
            q_inbox: d(self.q_inbox, prev.q_inbox),
            submit: d(self.submit, prev.submit),
            collect: d(self.collect, prev.collect),
            cap_retries: self.cap_retries.saturating_sub(prev.cap_retries),
            req: d(self.req, prev.req),
            req_hist: self.req_hist.since(&prev.req_hist),
            aps: d(self.aps, prev.aps),
            aps_hist: self.aps_hist.since(&prev.aps_hist),
            phases: self.phases.since(&prev.phases),
            ..*self
        }
    }
}

fn avg_us(a: &Acc) -> u64 {
    a.ns.checked_div(a.n).unwrap_or(0) / 1000
}

/// The OLD `vat[...]` fields, exactly as before 2026-11-16 (names, order, units, meaning):
/// `invals arrive->clear_avg_us max_us walks walk_avg_us gpu_avg_us plan_avg_us apply_avg_us
/// leaves host_calls apply_max_ms split_max_ms slow_applies slow_splits`.
#[must_use]
pub fn vat_old_fields(tm: &crate::vasmgr::VaTiming) -> String {
    let avg = |sum: u64, n: u64| sum.checked_div(n).unwrap_or(0) / 1000;
    format!(
        "invals={} arrive->clear_avg_us={} max_us={} walks={} walk_avg_us={} gpu_avg_us={} plan_avg_us={} apply_avg_us={} leaves={} host_calls={} apply_max_ms={} split_max_ms={} slow_applies={} slow_splits={}",
        tm.invals,
        avg(tm.inval_ns, tm.invals),
        tm.inval_ns_max / 1000,
        tm.walks,
        avg(tm.walk_ns, tm.walks),
        tm.gpu_us.checked_div(tm.walks).unwrap_or(0),
        avg(tm.plan_ns, tm.walks),
        avg(tm.apply_ns, tm.walks),
        tm.leaves_last,
        tm.host_calls,
        tm.apply_ns_max / 1_000_000,
        tm.split_ns_max / 1_000_000,
        tm.slow_applies,
        tm.slow_splits,
    )
}

/// The NEW `vat[...]` fields (appended after the old ones; `key=integer`, no brackets). Window
/// values unless named `*_max_*` or listed as boot-wide in [`VaProfCounters::since`].
#[must_use]
pub fn vat_new_fields(p: &VaProfCounters) -> String {
    let us = |ns: u64| ns / 1000;
    let mut s = format!(
        "q_inbox_n={} q_inbox_avg_us={} q_inbox_max_us={} submit_n={} submit_avg_us={} submit_max_us={} collect_n={} collect_avg_us={} collect_max_us={} cap_retries={} req_n={} req_avg_us={} req_p50_us={} req_p95_us={} req_p99_us={} req_max_ms={} aps_n={} aps_avg_us={} aps_p50_us={} aps_p95_us={} aps_p99_us={} aps_max_ms={}",
        p.q_inbox.n,
        avg_us(&p.q_inbox),
        us(p.q_inbox.max),
        p.submit.n,
        avg_us(&p.submit),
        us(p.submit.max),
        p.collect.n,
        avg_us(&p.collect),
        us(p.collect.max),
        p.cap_retries,
        p.req.n,
        avg_us(&p.req),
        p.req_hist.percentile_us(500),
        p.req_hist.percentile_us(950),
        p.req_hist.percentile_us(990),
        p.req.max / 1_000_000,
        p.aps.n,
        avg_us(&p.aps),
        p.aps_hist.percentile_us(500),
        p.aps_hist.percentile_us(950),
        p.aps_hist.percentile_us(990),
        p.aps.max / 1_000_000,
    );
    for ph in Phase::ALL {
        let a = p.phases.get(ph);
        s.push_str(&format!(
            " {0}_n={1} {0}_us={2} {0}_max_us={3}",
            ph.name(),
            a.n,
            us(a.ns),
            us(a.max)
        ));
    }
    s.push_str(&format!(
        " slow_reqs={} slow_aps={} windows_max={} slow_printed={} slow_suppressed={} ring_overwritten={}",
        p.slow_reqs,
        p.slow_aps,
        p.windows_max,
        p.slow_printed,
        p.slow_suppressed,
        p.ring_overwritten
    ));
    s
}

/// Phases this module formats (a compile-time check that the field list covers every phase).
const _: () = assert!(Phase::ALL.len() == COUNT);

#[cfg(test)]
mod tests {
    use super::*;

    /// The analysis regexes, in Rust: `vat\[(.*?)\]` (the first `]` after `vat[`), then
    /// `(\w+)=(\d+)` (a word run directly before `=`, then a digit run).
    pub(crate) fn parse_vat(line: &str) -> Vec<(String, u64)> {
        let Some(start) = line.find("vat[") else {
            return Vec::new();
        };
        let body = &line[start + 4..];
        let body = &body[..body.find(']').unwrap_or(body.len())];
        let bytes = body.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'=' {
                let mut k = i;
                while k > 0 && (bytes[k - 1].is_ascii_alphanumeric() || bytes[k - 1] == b'_') {
                    k -= 1;
                }
                let mut j = i + 1;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                if k < i && j > i + 1 {
                    out.push((
                        body[k..i].to_string(),
                        body[i + 1..j].parse().unwrap_or(u64::MAX),
                    ));
                    i = j;
                    continue;
                }
            }
            i += 1;
        }
        out
    }

    #[test]
    fn hist_buckets_are_log2_of_microseconds() {
        assert_eq!(bucket_of_ns(0), 0);
        assert_eq!(bucket_of_ns(1_999), 0);
        assert_eq!(bucket_of_ns(2_000), 1);
        assert_eq!(bucket_of_ns(3_999), 1);
        assert_eq!(bucket_of_ns(4_000), 2);
        assert_eq!(bucket_of_ns(1_023_999), 9);
        assert_eq!(bucket_of_ns(1_024_000), 10);
        assert_eq!(bucket_of_ns(u64::MAX), HIST_BUCKETS - 1);
        assert_eq!(bucket_upper_us(0), 2);
        assert_eq!(bucket_upper_us(9), 1024);
        assert_eq!(bucket_upper_us(HIST_BUCKETS - 1), 1 << 32);
        // Every bucket's samples are below its upper bound and at or above the previous one's.
        for i in 1..HIST_BUCKETS - 1 {
            let lo_ns = bucket_upper_us(i - 1) * 1000;
            assert_eq!(bucket_of_ns(lo_ns), i);
            assert_eq!(bucket_of_ns(bucket_upper_us(i) * 1000 - 1), i);
        }
    }

    #[test]
    fn hist_percentiles_are_the_upper_bound_of_the_ranked_bucket() {
        let mut h = Log2Hist::default();
        assert_eq!(h.percentile_us(500), 0);
        // 90 samples at 100 us (bucket 6: [64,128)), 9 at 600 ms (bucket 19), 1 at 3.4 s (bucket 21).
        for _ in 0..90 {
            h.record_ns(100_000);
        }
        for _ in 0..9 {
            h.record_ns(600_000_000);
        }
        h.record_ns(3_400_000_000);
        assert_eq!(h.count(), 100);
        assert_eq!(h.percentile_us(500), 128);
        assert_eq!(h.percentile_us(900), 128);
        assert_eq!(h.percentile_us(910), 1 << 20);
        assert_eq!(h.percentile_us(950), 1 << 20);
        assert_eq!(h.percentile_us(990), 1 << 20);
        assert_eq!(h.percentile_us(1000), 1 << 22);
    }

    #[test]
    fn hist_percentile_of_one_sample_is_its_bucket() {
        let mut h = Log2Hist::default();
        h.record_ns(1_500_000_000); // 1.5 s: bucket 20 ([2^20, 2^21) us)
        assert_eq!(h.percentile_us(10), 1 << 21);
        assert_eq!(h.percentile_us(990), 1 << 21);
        let mut g = h;
        g.record_ns(10_000);
        assert_eq!(g.since(&h).count(), 1);
        assert_eq!(g.since(&h).percentile_us(500), 16);
    }

    fn rec(t: u64) -> SlowRecord {
        SlowRecord {
            ticket: t,
            ..SlowRecord::default()
        }
    }

    #[test]
    fn ring_never_grows_past_its_size_and_counts_what_it_overwrote() {
        let mut r = SlowRing::default();
        assert!(r.is_empty());
        for t in 1..=(SLOW_RING as u64 + 10) {
            assert_eq!(r.push(rec(t)), t);
        }
        assert_eq!(r.len(), SLOW_RING);
        assert_eq!(r.pushed(), SLOW_RING as u64 + 10);
        assert_eq!(r.overwritten(), 10);
        let held = r.held();
        assert_eq!(held.len(), SLOW_RING);
        // oldest first: the first 10 were overwritten
        assert_eq!(held[0].1.ticket, 11);
        assert_eq!(held[0].0, 11);
        assert_eq!(held[SLOW_RING - 1].1.ticket, SLOW_RING as u64 + 10);
    }

    #[test]
    fn ring_marks_only_the_named_record_printed() {
        let mut r = SlowRing::default();
        let a = r.push(rec(1));
        let b = r.push(rec(2));
        r.mark_printed(b);
        let held = r.held();
        assert_eq!(held.len(), 2);
        assert_eq!((held[0].0, held[0].2), (a, false));
        assert_eq!((held[1].0, held[1].2), (b, true));
        // a record overwritten later is a fresh, unprinted one
        for t in 3..=(SLOW_RING as u64 + 2) {
            r.push(rec(t));
        }
        assert!(r.held().iter().all(|(s, _, p)| *p == (*s == b)));
        r.push(rec(99));
        assert!(r.held().iter().all(|(_, _, p)| !p));
    }

    #[test]
    fn dump_budget_bursts_refills_on_the_clock_and_caps_the_boot() {
        let s = 1_000_000_000u64;
        let mut d = DumpBudget::new(3, 4 * s, 5);
        assert!(d.admit(0));
        assert!(d.admit(0));
        assert!(d.admit(0));
        assert!(!d.admit(s)); // burst spent, no refill yet
        assert!(d.admit(4 * s)); // one refilled
        assert!(!d.admit(5 * s));
        assert!(d.admit(100 * s)); // refilled to the burst ...
        assert!(!d.admit(100 * s)); // ... but the boot cap (5) is reached
        assert!(!d.admit(1000 * s));
        assert_eq!((d.printed(), d.suppressed()), (5, 4));
    }

    #[test]
    fn dump_budget_prints_a_late_relaunch_after_an_early_storm() {
        let s = 1_000_000_000u64;
        let mut d = DumpBudget::default();
        let early = (0..1000).filter(|_| d.admit(10 * s)).count();
        assert_eq!(early, DUMP_BURST as usize);
        // ten minutes later (a relaunch): its first records are printed again
        assert!(d.admit(610 * s));
        assert!(d.printed() <= DUMP_CAP);
    }

    #[test]
    fn vat_old_fields_keep_their_names_order_and_meaning() {
        let tm = crate::vasmgr::VaTiming {
            invals: 4,
            inval_ns: 8_000_000,
            inval_ns_max: 3_000_000,
            walks: 2,
            walk_ns: 50_188_000,
            gpu_us: 46_230,
            plan_ns: 0,
            apply_ns: 3_626_352_000,
            leaves_last: 99_134,
            host_calls: 4125,
            apply_ns_max: 1_813_176_000,
            split_ns_max: 1_838_000_000,
            slow_applies: 83,
            slow_splits: 14,
            ..crate::vasmgr::VaTiming::default()
        };
        let line = format!(
            "kf3: x vat[{} {}] mem[a=1]",
            vat_old_fields(&tm),
            vat_new_fields(&tm.prof)
        );
        let got = parse_vat(&line);
        let want: [(&str, u64); 14] = [
            ("invals", 4),
            ("clear_avg_us", 2000),
            ("max_us", 3000),
            ("walks", 2),
            ("walk_avg_us", 25094),
            ("gpu_avg_us", 23115),
            ("plan_avg_us", 0),
            ("apply_avg_us", 1_813_176),
            ("leaves", 99_134),
            ("host_calls", 4125),
            ("apply_max_ms", 1813),
            ("split_max_ms", 1838),
            ("slow_applies", 83),
            ("slow_splits", 14),
        ];
        // the old fields come first, in their order, with their values
        for (i, (k, v)) in want.iter().enumerate() {
            assert_eq!((got[i].0.as_str(), got[i].1), (*k, *v), "field {i}");
        }
        // and no new field reuses an old name
        for (k, _) in &got[want.len()..] {
            assert!(!want.iter().any(|(o, _)| o == k), "{k} reuses an old name");
        }
        assert!(line.contains(
            "vat[invals=4 arrive->clear_avg_us=2000 max_us=3000 walks=2 walk_avg_us=25094 gpu_avg_us=23115 plan_avg_us=0 apply_avg_us=1813176 leaves=99134 host_calls=4125 apply_max_ms=1813 split_max_ms=1838 slow_applies=83 slow_splits=14 "
        ));
    }

    #[test]
    fn vat_new_fields_are_unique_integers_inside_the_block() {
        let mut p = VaProfCounters::default();
        p.req.add(1_200_000_000);
        p.req_hist.record_ns(1_200_000_000);
        p.phases.add(Phase::RmUnmap, 631_000_000);
        p.q_inbox.add(37_000_000);
        p.slow_reqs = 3;
        let body = vat_new_fields(&p);
        assert!(!body.contains('[') && !body.contains(']'));
        let got = parse_vat(&format!("vat[{body}]"));
        let n_fields = body.split(' ').count();
        assert_eq!(got.len(), n_fields, "every field parses as key=integer");
        let mut names: Vec<&str> = got.iter().map(|(k, _)| k.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), got.len(), "field names are unique");
        let get = |k: &str| got.iter().find(|(n, _)| n == k).map(|x| x.1);
        assert_eq!(get("req_n"), Some(1));
        assert_eq!(get("req_max_ms"), Some(1200));
        assert_eq!(get("req_p99_us"), Some(1 << 21));
        assert_eq!(get("rm_unmap_n"), Some(1));
        assert_eq!(get("rm_unmap_max_us"), Some(631_000));
        assert_eq!(get("q_inbox_avg_us"), Some(37_000));
        assert_eq!(get("slow_reqs"), Some(3));
    }

    #[test]
    fn counters_since_window_sums_but_keeps_boot_maxima() {
        let mut a = VaProfCounters::default();
        a.req.add(5_000_000);
        a.req_hist.record_ns(5_000_000);
        a.slow_reqs = 1;
        let mut b = a;
        b.req.add(2_000_000);
        b.req_hist.record_ns(2_000_000);
        b.cap_retries = 2;
        let d = b.since(&a);
        assert_eq!((d.req.n, d.req.ns, d.req.max), (1, 2_000_000, 5_000_000));
        assert_eq!(d.req_hist.count(), 1);
        assert_eq!(d.cap_retries, 2);
        assert_eq!(d.slow_reqs, 1);
    }

    #[test]
    fn slow_record_lines_carry_ids_and_every_phase() {
        let mut r = SlowRecord {
            kind: SlowKind::Apply,
            ticket: 47748,
            walk: 18973,
            key: 0x12,
            pdb: 0x8000,
            slot: 18,
            window: 5,
            apply_us: 3_234_000,
            ..SlowRecord::default()
        };
        r.phases.add(Phase::RmMap, 18_658_000);
        let l = r.line(7);
        for want in [
            "kind=apply",
            "n=7",
            "ticket=47748",
            "walk=18973",
            "key=0x12",
            "pdb=0x8000",
            "slot=18",
            "window=5",
            "rm_map_n=1 rm_map_us=18658 rm_map_max_us=18658",
            "reap_n=0",
        ] {
            assert!(l.contains(want), "{want} in {l}");
        }
        let t = SlowRecord {
            kind: SlowKind::Ticket,
            ticket: 9,
            windows: 3,
            q_inbox_us: 12,
            ..SlowRecord::default()
        }
        .line(8);
        assert!(
            t.contains("kind=ticket") && t.contains("windows=3") && t.contains("q_inbox_us=12")
        );
        assert!(!l.contains('[') && !t.contains('['));
    }

    #[test]
    fn dump_budget_never_exceeds_its_cap_over_a_long_boot() {
        let mut d = DumpBudget::default();
        // one slow record a second for ten hours
        let printed = (0..36_000u64)
            .filter(|t| d.admit(t * 1_000_000_000))
            .count() as u64;
        assert_eq!(printed, DUMP_CAP);
        assert_eq!(d.suppressed(), 36_000 - DUMP_CAP);
    }

    /// The old fields, byte for byte, against the 2026-11-13 format string of `device.rs` (copied
    /// here verbatim, with its arguments): the analysis scripts parse this text.
    #[test]
    fn vat_old_fields_match_the_2026_11_13_format_string() {
        let tm = crate::vasmgr::VaTiming {
            invals: 3,
            inval_ns: 9_000_000,
            inval_ns_max: 7_000_000,
            walks: 5,
            walk_ns: 125_000_000,
            gpu_us: 100_000,
            plan_ns: 5_000_000,
            apply_ns: 4_000_000_000,
            leaves_last: 262_144,
            host_calls: 99,
            apply_ns_max: 3_234_000_000,
            split_ns_max: 3_369_000_000,
            slow_applies: 73,
            slow_splits: 14,
            ..crate::vasmgr::VaTiming::default()
        };
        // `if n == 0 { 0 } else { sum / n / 1000 }` and `if walks == 0 { 0 } else { gpu_us / walks }`
        // in the original, written as checked divisions (the same values; the literal below pins them).
        let avg = |sum: u64, n: u64| sum.checked_div(n).unwrap_or(0) / 1000;
        let old = format!(
            "invals={} arrive->clear_avg_us={} max_us={} walks={} walk_avg_us={} gpu_avg_us={} plan_avg_us={} apply_avg_us={} leaves={} host_calls={} apply_max_ms={} split_max_ms={} slow_applies={} slow_splits={}",
            tm.invals,
            avg(tm.inval_ns, tm.invals),
            tm.inval_ns_max / 1000,
            tm.walks,
            avg(tm.walk_ns, tm.walks),
            tm.gpu_us.checked_div(tm.walks).unwrap_or(0),
            avg(tm.plan_ns, tm.walks),
            avg(tm.apply_ns, tm.walks),
            tm.leaves_last,
            tm.host_calls,
            tm.apply_ns_max / 1_000_000,
            tm.split_ns_max / 1_000_000,
            tm.slow_applies,
            tm.slow_splits,
        );
        assert_eq!(vat_old_fields(&tm), old);
        assert_eq!(
            old,
            "invals=3 arrive->clear_avg_us=3000 max_us=7000 walks=5 walk_avg_us=25000 gpu_avg_us=20000 plan_avg_us=1000 apply_avg_us=800000 leaves=262144 host_calls=99 apply_max_ms=3234 split_max_ms=3369 slow_applies=73 slow_splits=14"
        );
    }
}
