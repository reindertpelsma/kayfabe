//! ★★★ **Passthrough completion interrupts — the gate between a host non-stall edge and a guest
//! vector** (2026-10-08).
//!
//! A Passthrough twin's pushbuffer runs on the real engine unparsed, so the completion interrupt
//! it asks for (`NON_STALL_INTERRUPT`, a copy's `LAUNCH_DMA` with `INTERRUPT_TYPE_NON_BLOCKING`)
//! is raised by the HOST GPU and serviced by the HOST RM. What reaches kayfabe is one of the host
//! RM's GPU-wide non-stall notifiers, which carry no identity:
//!
//! - the engine's own notifier (`engineNonStallIntrNotify(RM_ENGINE_TYPE_COPY(n))`,
//!   `ogkm-595.91.07: kernel_ce.c:717`), when the host services that engine's notification;
//! - `NV2080_NOTIFIERS_FIFO_EVENT_MTHD`, which the host RM fires for EVERY host-driven engine's
//!   non-stall service under `bDefaultNonstallNotify` (`ogkm-595.91.07: intr.c:1210-1214`).
//!
//! `[measured 2026-10-08, rawclient --ce-interrupt at 9925108e, RTX 4070, bare metal,
//! traces/rawclient_ce_interrupt_20261008/bare_ce-interrupt_9925108e_run{1,2,3}.log]` a COPY0
//! copy's interrupt arrives on `FIFO_EVENT_MTHD` ONLY (CE0's own notifier stays silent: COPY0 is a
//! graphics CE on that die); a COPY2 copy's on both CE2 and `FIFO_EVENT_MTHD`. So a plane that
//! watches only engine notifiers never sees a COPY0 twin's completion — the guest's armed event
//! never fires (`guest_kf3_ce-interrupt_serial_9925108e.log`: COPY0 0/3).
//!
//! ## The rule (owner's design points, 2026-10-08)
//!
//! A host edge becomes a guest interrupt on an engine's guest vector only if **this VM** has a
//! live Passthrough twin on that engine **and** an outstanding submission there:
//!
//! - **fresh** — a doorbell of one of this VM's twins on the engine was counted since the previous
//!   edge this gate judged ([`PtGate::note_submit`] runs BEFORE the host doorbell store, so a
//!   completion can never be judged before its own doorbell was counted);
//! - **afterglow** — a fresh edge was seen less than the afterglow ago. ⊘ Without it, ANY edge in
//!   between (another tenant's, kayfabe's own Translated ring's) would consume the doorbell and
//!   the submission's real completion edge — later — would be dropped: a lost wakeup. The window
//!   bounds that: a completion is lost only if it lands more than the afterglow after the first
//!   edge that followed its doorbell AND no doorbell of this VM came in between.
//!
//! An edge with no twin, or with nothing outstanding, raises nothing — the guest learns nothing
//! about another tenant's work while it has none of its own in flight. ⊘ Never forged: the gate is
//! asked only by a worker holding a REAL host edge. ⊘ No allocation, no lock: atomics only.

use std::sync::atomic::{AtomicU64, Ordering};

/// The default afterglow, in microseconds (1 s). `KF3_PT_NSI_AFTERGLOW_MS` overrides it.
pub const DEFAULT_AFTERGLOW_US: u64 = 1_000_000;
/// The largest afterglow a setting may ask for (60 s).
pub const MAX_AFTERGLOW_US: u64 = 60_000_000;

/// The afterglow from `KF3_PT_NSI_AFTERGLOW_MS` (clamped to [`MAX_AFTERGLOW_US`]), else the default.
#[must_use]
pub fn afterglow_us_from(setting: Option<&str>) -> u64 {
    setting
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map_or(DEFAULT_AFTERGLOW_US, |ms| {
            ms.saturating_mul(1000).min(MAX_AFTERGLOW_US)
        })
}

/// What the gate decided for one host edge on one engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// No live Passthrough twin of this VM on the engine: not this VM's work.
    NoTwin,
    /// A twin lives but nothing of this VM is outstanding there: raised nothing.
    Idle,
    /// A doorbell was counted since the previous edge: raised.
    Fresh,
    /// No new doorbell, but a fresh edge was seen within the afterglow: raised.
    Afterglow,
}

impl Verdict {
    /// Whether this edge becomes a guest interrupt.
    #[must_use]
    pub fn raises(self) -> bool {
        matches!(self, Verdict::Fresh | Verdict::Afterglow)
    }
}

/// Per-engine state of this VM's Passthrough twins. Bounded: three words and four counters.
#[derive(Debug, Default)]
pub struct PtGate {
    /// Doorbells of this VM's Passthrough twins on the engine (vCPU / drainer side).
    submits: AtomicU64,
    /// The `submits` value the last judged edge saw (worker side).
    seen: AtomicU64,
    /// Until when (µs, the caller's clock) an edge without a new doorbell still raises.
    hot_until_us: AtomicU64,
    /// Edges judged.
    pub edges: AtomicU64,
    /// Edges raised because a doorbell was new.
    pub fresh: AtomicU64,
    /// Edges raised inside the afterglow.
    pub afterglow: AtomicU64,
    /// Edges with a live twin and nothing outstanding.
    pub idle: AtomicU64,
    /// Edges with no live twin.
    pub no_twin: AtomicU64,
}

impl PtGate {
    /// A doorbell of one of this VM's twins on the engine — call BEFORE the host doorbell store.
    /// One relaxed-release add: safe on a vCPU.
    pub fn note_submit(&self) {
        self.submits.fetch_add(1, Ordering::Release);
    }

    /// Doorbells counted so far.
    #[must_use]
    pub fn submits(&self) -> u64 {
        self.submits.load(Ordering::Relaxed)
    }

    /// Judge one REAL host edge: `live` = this VM's live Passthrough twins on the engine, `now_us`
    /// the caller's monotonic clock, `afterglow_us` the window. Safe from several workers at once
    /// (the doorbell consumption is one atomic swap).
    pub fn on_edge(&self, live: u64, now_us: u64, afterglow_us: u64) -> Verdict {
        self.edges.fetch_add(1, Ordering::Relaxed);
        let s = self.submits.load(Ordering::Acquire);
        // Consumed even with no twin: a retired twin's last doorbells must not make a later
        // twin's first idle edge look fresh.
        let prev = self.seen.swap(s, Ordering::AcqRel);
        let v = if live == 0 {
            Verdict::NoTwin
        } else if s != prev {
            self.hot_until_us
                .fetch_max(now_us.saturating_add(afterglow_us), Ordering::AcqRel);
            Verdict::Fresh
        } else if now_us < self.hot_until_us.load(Ordering::Acquire) {
            Verdict::Afterglow
        } else {
            Verdict::Idle
        };
        let c = match v {
            Verdict::NoTwin => &self.no_twin,
            Verdict::Idle => &self.idle,
            Verdict::Fresh => &self.fresh,
            Verdict::Afterglow => &self.afterglow,
        };
        c.fetch_add(1, Ordering::Relaxed);
        v
    }

    /// `edges=… fresh=… afterglow=… idle=… no_twin=… submits=…` — for the device's report lines.
    #[must_use]
    pub fn summary(&self) -> String {
        let o = Ordering::Relaxed;
        format!(
            "edges={} fresh={} afterglow={} idle={} no_twin={} submits={}",
            self.edges.load(o),
            self.fresh.load(o),
            self.afterglow.load(o),
            self.idle.load(o),
            self.no_twin.load(o),
            self.submits.load(o)
        )
    }
}

/// A set of guest vectors to raise for one edge — coalescing, so engines that share a vector
/// (a graphics CE announces on GR's) raise it once. Fixed size, no allocation: vectors `0..256`;
/// a larger one is refused (`insert` answers `false`) and counted by the caller.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct VectorSet([u64; 4]);

impl VectorSet {
    /// Add `v`; `false` if it is out of range or already present.
    pub fn insert(&mut self, v: u32) -> bool {
        let Some(w) = self.0.get_mut((v / 64) as usize) else {
            return false;
        };
        let bit = 1u64 << (v % 64);
        let new = *w & bit == 0;
        *w |= bit;
        new
    }

    /// Whether nothing is in the set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|&w| w == 0)
    }

    /// Every vector in the set, ascending.
    pub fn for_each(&self, mut f: impl FnMut(u32)) {
        for (i, &w) in self.0.iter().enumerate() {
            let mut bits = w;
            while bits != 0 {
                let b = bits.trailing_zeros();
                bits &= bits - 1;
                f(i as u32 * 64 + b);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AG: u64 = 1_000_000;

    #[test]
    fn an_edge_before_any_submission_raises_nothing() {
        let g = PtGate::default();
        assert_eq!(g.on_edge(1, 10, AG), Verdict::Idle);
        assert_eq!(g.idle.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_submission_then_its_edge_raises() {
        let g = PtGate::default();
        assert_eq!(g.on_edge(1, 10, AG), Verdict::Idle, "edge before the doorbell");
        g.note_submit();
        assert_eq!(g.on_edge(1, 20, AG), Verdict::Fresh);
        assert!(Verdict::Fresh.raises());
    }

    #[test]
    fn an_edge_with_no_outstanding_work_after_the_afterglow_raises_nothing() {
        let g = PtGate::default();
        g.note_submit();
        assert_eq!(g.on_edge(1, 100, AG), Verdict::Fresh);
        assert_eq!(g.on_edge(1, 100 + AG - 1, AG), Verdict::Afterglow);
        assert_eq!(g.on_edge(1, 100 + AG, AG), Verdict::Idle);
        assert!(!Verdict::Idle.raises());
    }

    #[test]
    fn a_foreign_edge_between_doorbell_and_completion_does_not_lose_the_completion() {
        // The lost-wakeup case: another tenant's edge consumes the doorbell first.
        let g = PtGate::default();
        g.note_submit();
        assert_eq!(g.on_edge(1, 50, AG), Verdict::Fresh, "the foreign edge (spurious, harmless)");
        assert_eq!(
            g.on_edge(1, 50 + 170, AG),
            Verdict::Afterglow,
            "the submission's own completion"
        );
    }

    #[test]
    fn no_live_twin_raises_nothing_and_consumes_the_retired_twins_doorbells() {
        let g = PtGate::default();
        g.note_submit();
        assert_eq!(g.on_edge(0, 10, AG), Verdict::NoTwin);
        // A twin born later: its first edge with no doorbell of its own is idle.
        assert_eq!(g.on_edge(1, 10 + 2 * AG, AG), Verdict::Idle);
    }

    #[test]
    fn another_vms_twin_raises_nothing_here() {
        // Two VMs are two planes, so two gates; only B's guest rang a doorbell.
        let (a, b) = (PtGate::default(), PtGate::default());
        b.note_submit();
        assert_eq!(a.on_edge(1, 10, AG), Verdict::Idle, "VM A has a twin but no work");
        assert_eq!(b.on_edge(1, 10, AG), Verdict::Fresh);
    }

    #[test]
    fn two_workers_judging_one_edge_raise_at_most_once_fresh() {
        let g = PtGate::default();
        g.note_submit();
        let v1 = g.on_edge(1, 10, AG);
        let v2 = g.on_edge(1, 10, AG);
        assert_eq!((v1, v2), (Verdict::Fresh, Verdict::Afterglow));
        assert_eq!(g.fresh.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn the_afterglow_setting_is_parsed_and_clamped() {
        assert_eq!(afterglow_us_from(None), DEFAULT_AFTERGLOW_US);
        assert_eq!(afterglow_us_from(Some("junk")), DEFAULT_AFTERGLOW_US);
        assert_eq!(afterglow_us_from(Some("250")), 250_000);
        assert_eq!(afterglow_us_from(Some("0")), 0);
        assert_eq!(afterglow_us_from(Some("99999999999999")), MAX_AFTERGLOW_US);
    }

    #[test]
    fn a_zero_afterglow_is_the_strict_doorbell_since_last_edge_rule() {
        let g = PtGate::default();
        g.note_submit();
        assert_eq!(g.on_edge(1, 10, 0), Verdict::Fresh);
        assert_eq!(g.on_edge(1, 10, 0), Verdict::Idle);
    }

    #[test]
    fn vectors_coalesce_and_out_of_range_is_refused() {
        let mut s = VectorSet::default();
        assert!(s.is_empty());
        assert!(s.insert(0));
        assert!(!s.insert(0), "GR0, CE0 and CE1 share vector 0: raised once");
        assert!(s.insert(65));
        assert!(!s.insert(256));
        assert!(!s.insert(u32::MAX));
        let mut got = Vec::new();
        s.for_each(|v| got.push(v));
        assert_eq!(got, vec![0, 65]);
    }
}
