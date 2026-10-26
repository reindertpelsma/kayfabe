//! One bounded, host-triggered first-153 snapshot; no guest reads or decisions.

use super::*;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

/// Bounded trigger input.
pub const TRIGGER_BYTES: usize = 512;
/// Apply history slots.
pub const APPLY_N: usize = 1024;
/// Explicit runs retained per apply, with omitted count.
pub const RUNS_N: usize = 8;
/// Dump lines, each capped at the existing LINE_MAX.
pub const LINES_MAX: usize = 12288;

/// Optional host-selected chid/VA-space; absent selectors mean all tracked channels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    /// Host channel id, low16 bits of the host token.
    pub chid: Option<u32>,
    /// Guest client<<32 | VA-space handle.
    pub space: Option<u64>,
}

/// Parse only `event=153 [chid=0x...] [space=0x...]`, no fake Xid or VA required.
pub fn parse(text: &[u8]) -> Option<Selection> {
    if text.len() > TRIGGER_BYTES {
        return None;
    }
    let text = std::str::from_utf8(text).ok()?;
    let mut words = text.split_ascii_whitespace();
    if words.next()? != "event=153" {
        return None;
    }
    let mut s = Selection::default();
    for word in words {
        if let Some(v) = word.strip_prefix("chid=0x") {
            if s.chid.is_some() {
                return None;
            }
            let v = u32::from_str_radix(v, 16).ok()?;
            if v > 0xffff {
                return None;
            }
            s.chid = Some(v);
        } else {
            let v = word.strip_prefix("space=0x")?;
            if s.space.is_some() {
                return None;
            }
            s.space = Some(u64::from_str_radix(v, 16).ok()?);
        }
    }
    Some(s)
}

/// Apply phases, on the same VA-manager thread as the host timing buckets.
#[derive(Clone, Copy, Debug, Default)]
pub struct Timing {
    /// Apply wall time only.
    pub wall_ns: u64,
    /// Actual diagnostic sleep wall time, included in wall_ns, not an extra cost.
    pub injected_ns: u64,
    /// Actual Linux thread CPU time during this apply; non-additive with RM wall time.
    pub cpu_ns: Option<u64>,
    /// Submit-to-collect wall time, including GPU and scheduling (not additive).
    pub walk_ns: u64,
    /// GPU-reported walk duration, separate from apply.
    pub gpu_us: u64,
    /// Same-thread RM/admission interval counters.
    pub costs: kf_host::diag::Snapshot,
}

/// Trusted-ring progress, copied from normal publication/observation, never polled for this dump.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    /// Actual successful private PUT publication time.
    pub publish_ns: u64,
    /// Absolute private descriptor publication sequence.
    pub put: u64,
    /// Last marker issued with that publication.
    pub marker: u64,
    /// Last ordinary hardware-word observation.
    pub observe_ns: u64,
    /// Raw private GET.
    pub get: u32,
    /// Raw private REFERENCE (tagged marker word).
    pub reference: u32,
}

/// One-shot poller: invalid/missing input never consumes the successful snapshot.
#[derive(Default)]
pub struct Poller {
    done: bool,
    last: Option<Instant>,
}

impl Poller {
    /// Bounded read of a regular host file; no guest/vCPU involvement.
    pub fn poll(&mut self, path: &Path, now: Instant) -> Option<Selection> {
        if self.done
            || self.last.is_some_and(|last| {
                now.saturating_duration_since(last) < Duration::from_millis(100)
            })
        {
            return None;
        }
        self.last = Some(now);
        // Host-owned path only. Reject non-regular inputs before opening; neither guest
        // bytes nor a guest handle can choose this path. The reader takes at most513bytes.
        if !std::fs::symlink_metadata(path).ok()?.file_type().is_file() {
            return None;
        }
        let file = std::fs::File::open(path).ok()?;
        if !file.metadata().ok()?.is_file() {
            return None;
        }
        let mut bytes = Vec::with_capacity(TRIGGER_BYTES + 1);
        file.take((TRIGGER_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .ok()?;
        let selection = parse(&bytes)?;
        self.done = true;
        Some(selection)
    }
}

#[derive(Clone, Debug)]
struct Apply {
    space: u64,
    walk: u64,
    start: u64,
    end: Option<u64>,
    nruns: usize,
    runs: Vec<DiffRun>,
    timing: Timing,
    injected_ms: u64,
}

/// Storage stays bounded independently of guest walks or run counts.
#[derive(Debug)]
pub(super) struct History {
    rows: Ring<Apply>,
}

impl Default for History {
    fn default() -> Self {
        Self {
            rows: Ring::new(APPLY_N),
        }
    }
}

impl Recorder {
    /// Begin an apply before any map/unmap/RM operation.
    pub fn ens3_begin(&self, space: u64, walk: u64, runs: &[DiffRun]) {
        lock(&self.ens3).rows.push(Apply {
            space,
            walk,
            start: self.now(),
            end: None,
            nruns: runs.len(),
            runs: runs.iter().take(RUNS_N).copied().collect(),
            timing: Timing::default(),
            injected_ms: 0,
        });
    }
    /// Record requested injection before sleeping, visible even to an in-flight snapshot.
    pub fn ens4_delay_requested(&self, space: u64, walk: u64, ms: u64) {
        if let Some(a) = lock(&self.ens3)
            .rows
            .buf
            .iter_mut()
            .find(|a| a.space == space && a.walk == walk)
        {
            a.injected_ms = ms;
        }
    }
    /// Complete its timing; preserves the pre-apply timestamp.
    pub fn ens3_end(&self, space: u64, walk: u64, timing: Timing) {
        let mut history = lock(&self.ens3);
        if let Some(a) = history
            .rows
            .buf
            .iter_mut()
            .find(|a| a.space == space && a.walk == walk)
        {
            a.end = Some(self.now());
            a.timing = timing;
        }
    }
    /// Copy the real relay's successful publication or hardware-word observation.
    pub fn ens3_progress(&self, token: u32, progress: Progress) {
        if let Some(chan) = self.chan_ref(token) {
            let mut chan = lock(&chan);
            if progress.publish_ns != 0 {
                if progress.marker > chan.progress.marker {
                    chan.packet.marker(progress.marker);
                    chan.exposure.marker(progress.marker, progress.publish_ns);
                    chan.ring.push(super::Rec {
                        t_ns: progress.publish_ns,
                        kind: super::Kind::MarkerPublished,
                        a: 0,
                        b: progress.marker,
                        c: progress.put,
                    });
                }
                chan.progress.publish_ns = progress.publish_ns;
                chan.progress.put = progress.put;
                chan.progress.marker = progress.marker;
            }
            if progress.observe_ns != 0 {
                chan.progress.observe_ns = progress.observe_ns;
                chan.progress.get = progress.get;
                chan.progress.reference = progress.reference;
            }
        }
    }
    /// No-Xid snapshot, selection optional, independently bounded from Xid/shutdown dumps.
    pub fn snapshot_153(&self, selection: Selection, t: u64) -> Vec<String> {
        if self.first_153_done.swap(true, Ordering::AcqRel) {
            return Vec::new();
        }
        // Copy fixed-size histories under their own locks, then release them BEFORE emitting.
        // No storage lock, GPU read, or lock taken by a vCPU is involved.
        let (chans, untracked) = {
            let st = lock(&self.st);
            let mut rows = Vec::new();
            for c in st.chans.values() {
                let c = lock(c);
                if selection.chid.is_some_and(|v| c.host_token & 0xffff != v)
                    || selection.space.is_some_and(|v| c.vas != v)
                {
                    continue;
                }
                rows.push((
                    c.host_token,
                    c.vas,
                    c.freed_ns,
                    c.held_now(t),
                    c.progress,
                    c.ring.overflow(),
                    c.last(DUMP_LAST),
                    c.exposure.stats(),
                    c.packet.class(c.hold.is_some() && c.pend_n > 0),
                ));
            }
            rows.sort_by_key(|r| r.0);
            (rows, st.untracked)
        };
        let (applies, overflow, kept) = {
            let h = lock(&self.ens3);
            let applies: Vec<_> = h
                .rows
                .newest_first()
                .filter(|a| {
                    selection.space.is_none_or(|v| a.space == v)
                        && (selection.chid.is_none() || chans.iter().any(|c| c.1 == a.space))
                })
                .take(128)
                .cloned()
                .collect();
            (applies, h.rows.overflow(), h.rows.len())
        };
        let mut out = vec![format!(
            "kf3: ENS3 FIRST153 t_ns={t} t0_unix_ms={} chans={} untracked={untracked} \
             apply_kept={kept} apply_overflow={overflow} apply_shown={} selector={selection:?}",
            self.t0_unix_ms,
            chans.len(),
            applies.len()
        )];
        for (token, space, freed, held, p, lost, records, exposure, class) in chans {
            out.push(format!(
                "kf3: ENS3 RING token={token:#x} space={space:#x} \
                freed_ns={freed:?} hold_age_ns={} held={} records_lost={lost}",
                held.hold_age_ns, held.count
            ));
            out.push(format!(
                "kf3: ENS3 PROGRESS token={token:#x} publish_ns={} put={} marker={} \
                observe_ns={} get={} reference={:#x} observe_age_ns={} \
                source=ordinary_reads_not_completion_receipt",
                p.publish_ns,
                p.put,
                p.marker,
                p.observe_ns,
                p.get,
                p.reference,
                t.saturating_sub(p.observe_ns)
            ));
            out.push(format!(
                "kf3: ENS4 PACKET token={token:#x} class={class} \
                scope=latest_observed_cohort affected_packet=unknown guest_delivery=unknown {}",
                exposure.summary()
            ));
            for r in records {
                out.push(rec_text(token, &r));
            }
        }
        for a in applies.iter().rev() {
            let [residual, rm, inv] = a.timing.costs.split(a.timing.wall_ns);
            out.push(format!(
                "kf3: ENS3 APPLY space={:#x} walk={} start_ns={} end_ns={:?} \
                finished={} runs={} omitted={} wall_ns={} cpu_residual_wall_ns={} rm_ns={} \
                invalidate_ns={} walk_ns={} gpu_walk_us={} thread_cpu_ns={:?} \
                injected_ms={} injected_wall_ns={}",
                a.space,
                a.walk,
                a.start,
                a.end,
                a.end.is_some(),
                a.nruns,
                a.nruns.saturating_sub(a.runs.len()),
                a.timing.wall_ns,
                residual,
                rm,
                inv,
                a.timing.walk_ns,
                a.timing.gpu_us,
                a.timing.cpu_ns,
                a.injected_ms,
                a.timing.injected_ns
            ));
            for r in &a.runs {
                out.push(format!(
                    "kf3: ENS3 RUN space={:#x} walk={} {} va={:#x} len={:#x} \
                    at={:#x} ap={} kind={} held={}",
                    a.space,
                    a.walk,
                    if r.unmap { "UNMAP" } else { "MAP" },
                    r.va,
                    r.len,
                    r.at,
                    r.ap,
                    r.kind,
                    r.held
                ));
            }
            for (name, c) in ["rm", "invalidate", "admission", "clone", "restore"]
                .iter()
                .zip(a.timing.costs.costs)
            {
                out.push(format!(
                    "kf3: ENS3 COST space={:#x} walk={} phase={name} calls={} \
                    ns={} work={} thread_last_call_ns={} thread_max_ns={}",
                    a.space, a.walk, c.calls, c.ns, c.work, c.last_ns, c.max_ns
                ));
            }
        }
        out.truncate(LINES_MAX - 1);
        out.iter_mut().for_each(|s| *s = cap(std::mem::take(s)));
        out.push(format!(
            "kf3: ENS3 END lines={} atomic_snapshot=false",
            out.len().saturating_add(1)
        ));
        out
    }
}
