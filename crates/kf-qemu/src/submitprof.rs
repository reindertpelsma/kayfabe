//! Optional direct USER-doorbell to next native-submit completion profilings.
//!
//! `KF3_PROF=1 KF3_PROF_USER_SUBMIT=1` enables this at construction. Nothing uses a
//! profiling to decide work or progress. A bounded token table holds the first pending
//! doorbell; the next native call consumes it. Coalesced kicks, unpaired calls, failures and
//! token-life changes are explicit. One doorbell can fan out to many native calls, so unpaired
//! continuations are never assigned zero latency or silently included in the histogram.

use crate::prof::Hist;
use std::sync::atomic::{AtomicU64, Ordering};

/// Native submission path, independent of guest privilege claims.
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    /// Authored guest-kernel translated work.
    Translated,
    /// Windows USER work through its private relay ring (a Translated token route).
    UserRelay,
    /// Direct Passthrough host doorbell.
    Passthrough,
}

impl Kind {
    fn index(self) -> usize {
        match self {
            Self::Translated => 0,
            Self::UserRelay => 1,
            Self::Passthrough => 2,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Translated => "translated",
            Self::UserRelay => "user_relay",
            Self::Passthrough => "passthrough",
        }
    }
}

#[derive(Default)]
struct Slot {
    first_ns: AtomicU64,
    born_ns: AtomicU64,
}

#[derive(Default)]
struct Stats {
    calls: AtomicU64,
    paired: AtomicU64,
    unpaired: AtomicU64,
    failed: AtomicU64,
    discarded: AtomicU64,
    latency: Hist,
}

/// A profiling ticket captured immediately before the native doorbell call.
/// It grants no product permission and owns no guest or native resource.
pub struct Ticket {
    idx: u32,
    kind: Kind,
    first_ns: u64,
    born_ns: u64,
    enabled: bool,
}

/// Read-only cumulative snapshot. Histogram quantiles are bucket upper bounds.
pub struct Snapshot {
    /// Native calls including failed calls.
    pub calls: u64,
    /// Successful calls with a pending doorbell from this token life.
    pub paired: u64,
    /// Successful calls without a pending doorbell, including fanout/continuations.
    pub unpaired: u64,
    /// Native calls that returned an error.
    pub failed: u64,
    /// Invalid token, old-life ticket or backwards clock, excluded from the histogram.
    pub discarded: u64,
    /// Histogram line in the existing profiler's units and bucket semantics.
    pub latency: String,
}

/// Fixed-size, allocation-free-on-use instrumentation. The vCPU takes no lock and never
/// spins on a failed comparison. With the flag off there are no token slots or clock reads.
pub struct SubmitProfile {
    enabled: bool,
    slots: Box<[Slot]>,
    stats: [Stats; 3],
    coalesced: AtomicU64,
}

impl SubmitProfile {
    /// Construct from a prevalidated host token-table size; no guest-controlled allocation.
    pub fn new(enabled: bool, slots: usize) -> Self {
        Self {
            enabled,
            slots: (0..if enabled { slots } else { 0 })
                .map(|_| Slot::default())
                .collect(),
            stats: std::array::from_fn(|_| Stats::default()),
            coalesced: AtomicU64::new(0),
        }
    }

    /// Read both profiling flags once, independent of prof::init construction order.
    pub fn from_env(slots: usize) -> Self {
        let base = std::env::var("KF3_PROF").ok();
        let extra = std::env::var("KF3_PROF_USER_SUBMIT").ok();
        Self::new(flags_enabled(base.as_deref(), extra.as_deref()), slots)
    }

    /// Cached profiling-only switch.
    #[inline]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Allocated token slots, zero when disabled.
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// Start a new token life before publishing its route. Old late stamps predate `born_ns`.
    pub fn reset(&self, idx: u32, now: u64) {
        if let Some(slot) = self.slots.get(idx as usize) {
            slot.born_ns.store(now, Ordering::Release);
            slot.first_ns.store(0, Ordering::Release);
        }
    }

    /// Observe a guest doorbell BEFORE publishing work to another thread.
    /// One CAS attempt preserves the first accepted pending stamp; no retry/spin loop.
    pub fn doorbell(&self, idx: u32, now: u64) {
        let Some(slot) = self.slots.get(idx as usize) else {
            return;
        };
        if now == 0 || now < slot.born_ns.load(Ordering::Acquire) {
            return;
        }
        if slot
            .first_ns
            .compare_exchange(0, now, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            self.coalesced.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Consume a pending stamp immediately BEFORE a native call, so a concurrent new
    /// doorbell belongs to a later call. A zero stamp remains explicitly unpaired.
    pub fn begin(&self, idx: u32, kind: Kind) -> Ticket {
        let slot = self.slots.get(idx as usize);
        Ticket {
            idx,
            kind,
            born_ns: slot.map_or(0, |s| s.born_ns.load(Ordering::Acquire)),
            first_ns: slot.map_or(0, |s| s.first_ns.swap(0, Ordering::AcqRel)),
            enabled: self.enabled,
        }
    }

    /// Complete only the profiling after the real native call returns.
    pub fn finish(&self, ticket: Ticket, now: u64, ok: bool) {
        if !ticket.enabled {
            return;
        }
        let stats = &self.stats[ticket.kind.index()];
        stats.calls.fetch_add(1, Ordering::Relaxed);
        if !ok {
            stats.failed.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let valid_life = self.slots.get(ticket.idx as usize).is_some_and(|slot| {
            slot.born_ns.load(Ordering::Acquire) == ticket.born_ns
                && (ticket.first_ns == 0 || ticket.first_ns >= ticket.born_ns)
        });
        if !valid_life || now < ticket.first_ns {
            stats.discarded.fetch_add(1, Ordering::Relaxed);
        } else if ticket.first_ns == 0 {
            stats.unpaired.fetch_add(1, Ordering::Relaxed);
        } else {
            stats.paired.fetch_add(1, Ordering::Relaxed);
            stats.latency.add(now.saturating_sub(ticket.first_ns));
        }
    }

    /// Observe one native call while preserving its exact original result.
    pub fn measure<T, E>(
        &self,
        idx: u32,
        kind: Kind,
        native: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, E> {
        if !self.enabled {
            return native();
        }
        let ticket = self.begin(idx, kind);
        let result = native();
        self.finish(ticket, crate::prof::now_ns(), result.is_ok());
        result
    }

    /// Reconstitute a callback ticket captured by the native-ring observer.
    pub fn finish_stamp(&self, idx: u32, stamp: (u64, u64), now: u64, ok: bool) {
        self.finish(
            Ticket {
                idx,
                kind: Kind::Translated,
                first_ns: stamp.0,
                born_ns: stamp.1,
                enabled: self.enabled,
            },
            now,
            ok,
        );
    }

    /// Coalesced guest doorbells across all paths, not additional native submissions.
    pub fn coalesced(&self) -> u64 {
        self.coalesced.load(Ordering::Relaxed)
    }

    /// Snapshot for diagnostics; it is never a scheduling input.
    pub fn snapshot(&self, kind: Kind) -> Snapshot {
        let stats = &self.stats[kind.index()];
        Snapshot {
            calls: stats.calls.load(Ordering::Relaxed),
            paired: stats.paired.load(Ordering::Relaxed),
            unpaired: stats.unpaired.load(Ordering::Relaxed),
            failed: stats.failed.load(Ordering::Relaxed),
            discarded: stats.discarded.load(Ordering::Relaxed),
            latency: stats.latency.line(),
        }
    }

    /// Printed only by a background reporter, never on a vCPU.
    pub fn lines(&self) -> Vec<String> {
        if !self.enabled {
            return Vec::new();
        }
        [Kind::Translated, Kind::UserRelay, Kind::Passthrough]
            .into_iter()
            .map(|kind| {
                let s = self.snapshot(kind);
                let bins = self.stats[kind.index()]
                    .latency
                    .counts_snapshot()
                    .into_iter()
                    .enumerate()
                    .filter(|(_, n)| *n != 0)
                    .map(|(i, n)| format!("{i}:{n}"))
                    .collect::<Vec<_>>()
                    .join(",");
                format!(
                    "kf3: PROF user_submit path={} calls={} paired={} unpaired={} \
                     failed={} discarded={} coalesced_total={} {} bins={bins}",
                    kind.name(),
                    s.calls,
                    s.paired,
                    s.unpaired,
                    s.failed,
                    s.discarded,
                    self.coalesced(),
                    s.latency,
                )
            })
            .collect()
    }
}

impl Ticket {
    /// Correlation fields for an optional native-ring callback.
    pub fn stamp(&self) -> (u64, u64) {
        (self.first_ns, self.born_ns)
    }
}

/// The profiling arm requires both explicit flags; construction can precede
/// prof::init.
pub fn flags_enabled(base: Option<&str>, extra: Option<&str>) -> bool {
    matches!(base, Some("1" | "on")) && extra == Some("1")
}
