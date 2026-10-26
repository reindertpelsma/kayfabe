//! Bounded gate-held publication to SET_REF completion-observation association.
use super::{Recorder, lock};
use std::collections::VecDeque;
/// Per-channel and per-boot diagnostic totals, never decision inputs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Maximum when an entry was already announced at its hold.
    pub known_ns: Option<u64>,
    /// Proxy from first observed gate hold when packet arrival was not observed.
    pub proxy_ns: Option<u64>,
    /// Completed marker-covered gated batches.
    pub completed: u64,
    /// Samples lost to bounds, retirement or invalid sequence/time.
    pub lost: u64,
    /// Batches still waiting for a covering marker/observation.
    pub pending: u64,
    /// Oldest pending start on the shared monotonic clock.
    pub oldest_ns: Option<u64>,
}
impl Stats {
    /// Worst observed interval, or unavailable with no matched receipt.
    pub fn worst(self) -> Option<u64> {
        self.known_ns.max(self.proxy_ns)
    }
    /// Fold channel totals, retaining the maximum rather than averaging.
    pub fn fold(&mut self, other: Self) {
        self.known_ns = self.known_ns.max(other.known_ns);
        self.proxy_ns = self.proxy_ns.max(other.proxy_ns);
        self.completed = self.completed.saturating_add(other.completed);
        self.lost = self.lost.saturating_add(other.lost);
        self.pending = self.pending.saturating_add(other.pending);
        self.oldest_ns = match (self.oldest_ns, other.oldest_ns) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }
    /// Status fragment with explicit missing data and unknown-arrival coverage.
    pub fn summary(self) -> String {
        let ns = self.worst().map_or("NA".into(), |n| n.to_string());
        let kind = if self.worst().is_none() {
            "unavailable"
        } else if self.proxy_ns >= self.known_ns {
            "gate_start_proxy"
        } else {
            "announced_observation"
        };
        format!(
            "ens4_worst_gate_apply_complete_ns={ns} ens4_metric={kind} \
            ens4_completed_batches={} ens4_pending_batches={} ens4_association_lost={} \
            ens4_oldest_pending_ns={:?}",
            self.completed, self.pending, self.lost, self.oldest_ns
        )
    }
}
#[derive(Clone, Copy, Debug)]
struct Candidate {
    start: u64,
    proxy: bool,
}
#[derive(Debug)]
struct Flight {
    marker: u64,
    published: u64,
    candidate: Candidate,
}
/// Fixed-budget observer; marker counts come from successful actual publication/Marks::seen.
#[derive(Debug)]
pub struct Exposure {
    cap: usize,
    open_hold: Option<u64>,
    carry: Option<u64>,
    building: Option<Candidate>,
    pending: VecDeque<Flight>,
    issued: u64,
    seen: u64,
    retired: bool,
    totals: Stats,
}
impl Exposure {
    /// Explicit cap for tiny tests; production chooses a512-entry diagnostic budget.
    pub fn new(cap: usize) -> Self {
        Self {
            cap: cap.min(512),
            open_hold: None,
            carry: None,
            building: None,
            pending: VecDeque::new(),
            issued: 0,
            seen: 0,
            retired: false,
            totals: Stats::default(),
        }
    }
    /// First gate call of a hold; repeated calls never reset its start.
    pub fn gate(&mut self, t: u64) {
        if self.retired || self.open_hold.is_some() {
            return;
        }
        self.open_hold = Some(t);
        self.carry = Some(self.carry.map_or(t, |old| old.min(t)));
    }
    /// Gate opens without changing any publication/receipt state.
    pub fn pass(&mut self) {
        self.open_hold = None;
    }
    /// Actual descriptor publication, with an optional observed announcement.
    pub fn published(&mut self, t: u64, announced: Option<u64>, held: bool) {
        if self.retired {
            return;
        }
        let candidate = if held {
            announced
                .map(|start| Candidate {
                    start,
                    proxy: false,
                })
                .or_else(|| self.carry.map(|start| Candidate { start, proxy: true }))
        } else {
            self.carry.map(|start| Candidate { start, proxy: true })
        };
        self.carry = None;
        let Some(c) = candidate else { return };
        if c.start > t {
            self.omitted();
            return;
        }
        self.building = Some(match self.building {
            Some(old) => Candidate {
                start: old.start.min(c.start),
                proxy: old.proxy || c.proxy,
            },
            None => c,
        });
    }
    /// No pending guest work after a gate: do not attach the hold to a later idle epoch.
    pub fn idle(&mut self) {
        if self.open_hold.is_none() {
            self.carry = None;
        }
    }
    /// Successful SET_REF descriptor publication; it covers preceding publications only.
    pub fn marker(&mut self, seq: u64, t: u64) {
        if self.retired || seq <= self.issued {
            return;
        }
        self.issued = seq;
        let Some(candidate) = self.building.take() else {
            return;
        };
        if self.cap == 0 {
            self.omitted();
            return;
        }
        if self.pending.len() == self.cap {
            self.pending.pop_front();
            self.omitted();
        }
        self.pending.push_back(Flight {
            marker: seq,
            published: t,
            candidate,
        });
    }
    /// Validated ordinary marker observation, not a GET/IRQ count.
    pub fn observed(&mut self, seen: u64, t: u64) {
        if self.retired || seen <= self.seen {
            return;
        }
        if seen > self.issued {
            self.omitted();
            return;
        }
        self.seen = seen;
        while self.pending.front().is_some_and(|f| f.marker <= seen) {
            let Some(f) = self.pending.pop_front() else {
                break;
            };
            if t < f.published || t < f.candidate.start {
                self.omitted();
                continue;
            }
            let ns = t.saturating_sub(f.candidate.start);
            let max = if f.candidate.proxy {
                &mut self.totals.proxy_ns
            } else {
                &mut self.totals.known_ns
            };
            *max = Some(max.unwrap_or(0).max(ns));
            self.totals.completed = self.totals.completed.saturating_add(1);
        }
    }
    /// A diagnostic publication/association could not be tracked.
    pub fn omitted(&mut self) {
        self.totals.lost = self.totals.lost.saturating_add(1);
    }
    /// Free/reset invalidates uncompleted association, while retaining completed totals.
    pub fn retire(&mut self) {
        let pending = self.stats().pending;
        self.totals.lost = self.totals.lost.saturating_add(pending);
        self.pending.clear();
        self.building = None;
        self.carry = None;
        self.open_hold = None;
        self.retired = true;
    }
    /// Bounded diagnostic state snapshot.
    pub fn stats(&self) -> Stats {
        let mut out = self.totals;
        out.pending = u64::try_from(self.pending.len())
            .unwrap_or(u64::MAX)
            .saturating_add(u64::from(self.building.is_some()));
        out.oldest_ns = self
            .pending
            .iter()
            .map(|f| f.candidate.start)
            .chain(self.building.iter().map(|c| c.start))
            .min();
        out
    }
}
impl Recorder {
    /// Fold actual marker publication and a validated ordinary observation at turn end.
    pub fn ens4_turn(&self, token: u32, marker: u64, seen: u64, observe_ns: u64, t: u64) {
        if let Some(c) = self.chan_ref(token) {
            let mut c = lock(&c);
            c.exposure.marker(marker, t);
            if seen > c.exposure.seen && seen <= marker {
                c.ring.push(super::Rec {
                    t_ns: observe_ns,
                    kind: super::Kind::MarkerSeen,
                    a: 0,
                    b: seen,
                    c: 0,
                });
            }
            c.packet.seen(seen);
            c.exposure.observed(seen, observe_ns);
        }
    }
    /// Per-boot status; known absence is NA, never a fabricated zero latency.
    pub fn ens4_summary(&self) -> String {
        let Ok(st) = self.st.try_lock() else {
            return "ens4_worst_gate_apply_complete_ns=NA ens4_metric=busy".into();
        };
        let mut total = st.evicted.exposure;
        let mut open_age = 0;
        let now = self.now();
        for c in st.chans.values() {
            let Ok(c) = c.try_lock() else {
                return "ens4_worst_gate_apply_complete_ns=NA ens4_metric=busy".into();
            };
            total.fold(c.exposure.stats());
            open_age = open_age.max(c.held_now(now).hold_age_ns);
        }
        if let Some(start) = total.oldest_ns {
            open_age = open_age.max(now.saturating_sub(start));
        }
        format!(
            "{} ens4_open_age_ns={open_age} ens4_untracked={} \
            ens4_receipt=tagged_SET_REF_implicit_idle",
            total.summary(),
            st.untracked
        )
    }
}
