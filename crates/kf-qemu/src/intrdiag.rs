//! Optional interrupt census. Compiled out without `diag-intr`; runtime opt-in is
//! `KF3_DIAG_INTR=1`. No observation participates in an interrupt decision.
//!
//! Host edges are dataless fd readiness notifications, potentially coalesced by RM;
//! the completion fd includes host CE as well as FIFO_EVENT_MTHD. Vector 512 means
//! unmapped, not a GPU vector. Logical raise counts include each contributing vector
//! of a tree-unmask message; physical eventfd writes are separately counted on v0.
//! Reads count only existing read exits. No read trap is installed by this observer.
//!
//! An ISR cycle is the first message through the next changing TOP_EN_SET (root
//! unmask), the Windows tree protocol. `stranded` counts observed enabled pending
//! flights older than 1 ms without a first tree write, not a proof of lost hardware
//! work. Snapshots are bounded, independently atomic observations, not a transaction.

use kf_trap::cpuintr::{CpuIntr, MAX_LEAF, Raise, Reg};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

const VECTORS: usize = MAX_LEAF * 32;
const BINS: usize = VECTORS + 1;
const CLASSES: [&str; 7] = [
    "en_set",
    "en_clear",
    "top_set",
    "top_clear",
    "w1c",
    "trigger",
    "top",
];
const SOURCES: [&str; 9] = [
    "engine", "fifo", "gr_relay", "ce_relay", "display", "gsp", "tick", "other", "tree",
];
const FIELDS: usize = NAMES.len() + 6 + 3 * BINS + SOURCES.len() + 2 * CLASSES.len();

macro_rules! metrics {
    ($($variant:ident => $name:literal),* $(,)?) => {
        #[derive(Clone, Copy)]
        enum Metric { $($variant),* }
        const NAMES: &[&str] = &[$($name),*];
    };
}

metrics! {
    Messages => "messages",
    Held => "held",
    OutOfRange => "out_of_range",
    WasAsserted => "was_asserted",
    Merges => "merge_candidates",
    Cycles => "isr_cycles",
    CycleNs => "isr_ns",
    CycleStarts => "cycle_starts",
    FirstWrites => "first_tree_writes",
    FirstWriteNs => "first_write_ns",
    MessageGaps => "message_gaps",
    MessageGapNs => "message_gap_ns",
    Stranded => "stranded",
    Eventfd => "eventfd_v0",
    EventfdErrors => "eventfd_errors",
    EngineEdges => "edge_engine",
    FifoEdges => "edge_fifo",
    Resets => "resets",
}

/// Authored marker publication site, never classified from guest command bytes.
#[derive(Clone, Copy)]
pub enum Marker {
    /// SYNC_WAIT gate's tagged reference.
    Gate,
    /// End-of-batch / private-credit reference.
    Batch,
    /// Staged trigger's tagged reference.
    Staged,
}

/// Origin of a latch request, independently of its eventual guest delivery.
#[derive(Clone, Copy)]
pub enum Source {
    /// Dedicated host engine notifier fd.
    Engine,
    /// Shared FIFO / asynchronous host-CE completion fd.
    Fifo,
    /// Translated GR retirement.
    GrRelay,
    /// Translated CE retirement.
    CeRelay,
    /// Display engine.
    Display,
    /// GSP message.
    Gsp,
    /// Optional pacer's trailing message.
    Tick,
    /// Other callers of the public latch entry.
    Other,
    /// Guest tree write, including loopback trigger and wake-on-unmask.
    Tree,
}

/// Fixed-size atomic observer; no locks, allocation, or text emission in event hooks.
pub struct Census {
    born: Instant,
    scalar: [AtomicU64; NAMES.len()],
    markers: [AtomicU64; 6],
    edges: [[AtomicU64; BINS]; 2],
    raises: [AtomicU64; BINS],
    sources: [AtomicU64; SOURCES.len()],
    writes: [AtomicU64; CLASSES.len()],
    reads: [AtomicU64; CLASSES.len()],
    flight: AtomicU64,
    reported_flight: AtomicU64,
    cycle: AtomicU64,
    previous_message: AtomicU64,
    status_previous: [AtomicU64; FIELDS],
    status_at: AtomicU64,
}

fn bump(counters: &[AtomicU64], index: usize, value: u64) {
    if let Some(counter) = counters.get(index) {
        counter.fetch_add(value, Ordering::Relaxed);
    }
}

fn class(reg: Reg) -> usize {
    match reg {
        Reg::LeafEnSet(_) => 0,
        Reg::LeafEnClear(_) => 1,
        Reg::TopEnSet => 2,
        Reg::TopEnClear => 3,
        Reg::Leaf(_) => 4,
        Reg::Trigger => 5,
        Reg::Top => 6,
    }
}

fn pending(tree: &CpuIntr, leaf: usize) -> u32 {
    if tree.read(Reg::TopEnSet) & (1 << (leaf / 2)) == 0 {
        return 0;
    }
    tree.read(Reg::Leaf(leaf)) & tree.read(Reg::LeafEnSet(leaf))
}

impl Default for Census {
    fn default() -> Self {
        Self::new()
    }
}

impl Census {
    /// Allocate only for the exact runtime opt-in, without mutating the environment.
    #[must_use]
    pub fn from_setting(setting: Option<&str>) -> Option<Self> {
        (setting == Some("1")).then(Self::new)
    }

    /// Empty census, also used by the deterministic simulation.
    #[must_use]
    pub fn new() -> Self {
        Self {
            born: Instant::now(),
            scalar: std::array::from_fn(|_| AtomicU64::new(0)),
            markers: std::array::from_fn(|_| AtomicU64::new(0)),
            edges: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU64::new(0))),
            raises: std::array::from_fn(|_| AtomicU64::new(0)),
            sources: std::array::from_fn(|_| AtomicU64::new(0)),
            writes: std::array::from_fn(|_| AtomicU64::new(0)),
            reads: std::array::from_fn(|_| AtomicU64::new(0)),
            flight: AtomicU64::new(0),
            reported_flight: AtomicU64::new(0),
            cycle: AtomicU64::new(0),
            previous_message: AtomicU64::new(0),
            status_previous: std::array::from_fn(|_| AtomicU64::new(0)),
            status_at: AtomicU64::new(0),
        }
    }

    /// Monotonic census-relative timestamp. Called only inside enabled hooks.
    #[must_use]
    pub fn now_ns(&self) -> u64 {
        u64::try_from(self.born.elapsed().as_nanos()).unwrap_or(u64::MAX - 1)
    }

    fn add(&self, metric: Metric, value: u64) {
        bump(&self.scalar, metric as usize, value);
    }

    /// Count after the authored marker's PUT publication succeeds.
    pub fn marker(&self, kind: Marker, nsi: bool) {
        let index = match kind {
            Marker::Gate => 0,
            Marker::Batch => 2,
            Marker::Staged => 4,
        } + usize::from(!nsi);
        bump(&self.markers, index, 1);
    }

    /// Count every received fd notification, including unarmed / unmapped ones.
    pub fn host_edge(&self, source: Source, vector: Option<u32>) {
        let (index, metric) = match source {
            Source::Engine => (0, Metric::EngineEdges),
            Source::Fifo => (1, Metric::FifoEdges),
            _ => return,
        };
        let vector = vector.map_or(VECTORS, |v| (v as usize).min(VECTORS));
        if let Some(row) = self.edges.get(index) {
            bump(row, vector, 1);
        }
        self.add(metric, 1);
    }

    fn message(&self, now: u64) {
        self.add(Metric::Messages, 1);
        let stamp = now.saturating_add(1);
        if self
            .flight
            .compare_exchange(0, stamp, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            self.add(Metric::Merges, 1);
        }
        if self
            .cycle
            .compare_exchange(0, stamp, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.add(Metric::CycleStarts, 1);
        }
        let previous = self.previous_message.swap(stamp, Ordering::AcqRel);
        if previous != 0 {
            self.add(Metric::MessageGaps, 1);
            self.add(Metric::MessageGapNs, stamp.saturating_sub(previous));
        }
    }

    /// Same latch and return value as production; the observer never suppresses a message.
    pub fn latch(&self, tree: &CpuIntr, vector: u32, source: Source, now: u64) -> Raise {
        self.sample(tree, now);
        bump(&self.sources, source as usize, 1);
        let leaf = vector as usize / 32;
        if leaf < MAX_LEAF && pending(tree, leaf) & (1 << (vector % 32)) != 0 {
            self.add(Metric::WasAsserted, 1);
        }
        let raise = tree.latch(vector);
        match raise {
            Raise::Message => {
                bump(&self.raises, (vector as usize).min(VECTORS), 1);
                self.message(now);
            }
            Raise::None => self.add(Metric::Held, 1),
            Raise::OutOfRange => self.add(Metric::OutOfRange, 1),
        }
        raise
    }

    /// Same synchronous tree write as production, with atomic accounting around it.
    pub fn write(&self, tree: &CpuIntr, reg: Reg, value: u32, now: u64) -> Raise {
        self.sample(tree, now);
        bump(&self.writes, class(reg), 1);
        let stamp = now.saturating_add(1);
        let flight = self.flight.swap(0, Ordering::AcqRel);
        if flight != 0 {
            self.add(Metric::FirstWrites, 1);
            self.add(Metric::FirstWriteNs, stamp.saturating_sub(flight));
        }
        let previous_enable = match reg {
            Reg::LeafEnSet(_) | Reg::TopEnSet => tree.read(reg),
            _ => 0,
        };
        let raise = tree.write(reg, value);
        if reg == Reg::TopEnSet && value & !previous_enable != 0 {
            let start = self.cycle.swap(0, Ordering::AcqRel);
            if start != 0 {
                self.add(Metric::Cycles, 1);
                self.add(Metric::CycleNs, stamp.saturating_sub(start));
            }
        }
        if raise == Raise::Message {
            bump(&self.sources, Source::Tree as usize, 1);
            let mut attributed = false;
            for leaf in 0..MAX_LEAF {
                let bits = match reg {
                    Reg::Trigger if leaf == (value & 0xfff) as usize / 32 => 1 << (value % 32),
                    Reg::LeafEnSet(i) if i == leaf => {
                        pending(tree, leaf) & value & !previous_enable
                    }
                    Reg::TopEnSet if value & !previous_enable & (1 << (leaf / 2)) != 0 => {
                        pending(tree, leaf)
                    }
                    _ => 0,
                };
                for bit in 0..32 {
                    if bits & (1 << bit) != 0 {
                        bump(&self.raises, leaf * 32 + bit, 1);
                        attributed = true;
                    }
                }
            }
            if !attributed {
                bump(&self.raises, VECTORS, 1);
            }
            self.message(now);
        }
        raise
    }

    /// Count an already-trapped tree read, without changing its value or trapping more reads.
    pub fn read(&self, reg: Reg) {
        bump(&self.reads, class(reg), 1);
    }

    /// Actual eventfd outcome; a logical raise is not proof that KVM accepted an eventfd write.
    pub fn delivery(&self, success: bool) {
        self.add(
            if success {
                Metric::Eventfd
            } else {
                Metric::EventfdErrors
            },
            1,
        );
    }

    /// Check the section-4 overdue predicate on hooks and on the existing status heartbeat.
    pub fn sample(&self, tree: &CpuIntr, now: u64) {
        let flight = self.flight.load(Ordering::Acquire);
        if flight == 0 || now.saturating_add(1).saturating_sub(flight) <= 1_000_000 {
            return;
        }
        if !(0..MAX_LEAF).any(|leaf| pending(tree, leaf) != 0) {
            return;
        }
        if self.flight.load(Ordering::Acquire) != flight {
            return;
        }
        if self.reported_flight.swap(flight, Ordering::AcqRel) != flight {
            self.add(Metric::Stranded, 1);
        }
    }

    /// Reset only temporal state, keeping evidence and its reset boundary in cumulative counts.
    pub fn reset(&self) {
        self.flight.store(0, Ordering::Release);
        self.reported_flight.store(0, Ordering::Release);
        self.cycle.store(0, Ordering::Release);
        self.previous_message.store(0, Ordering::Release);
        self.add(Metric::Resets, 1);
    }

    /// Bounded snapshot for the status thread, never for a vCPU or a delivery decision.
    #[must_use]
    pub fn snapshot(&self, at_ns: u64) -> Snapshot {
        let mut values = BTreeMap::new();
        let mut copy = |names: Vec<String>, counters: &[AtomicU64]| {
            for (name, counter) in names.into_iter().zip(counters) {
                values.insert(name, counter.load(Ordering::Relaxed));
            }
        };
        copy(
            NAMES.iter().map(|s| (*s).to_owned()).collect(),
            &self.scalar,
        );
        copy(
            ["gate", "batch", "staged"]
                .iter()
                .flat_map(|kind| [format!("marker_{kind}_nsi"), format!("marker_{kind}_plain")])
                .collect(),
            &self.markers,
        );
        copy(
            SOURCES.iter().map(|s| format!("source_{s}")).collect(),
            &self.sources,
        );
        copy(
            CLASSES.iter().map(|s| format!("write_{s}")).collect(),
            &self.writes,
        );
        copy(
            CLASSES.iter().map(|s| format!("read_{s}")).collect(),
            &self.reads,
        );
        copy(
            (0..BINS).map(|v| format!("raise_v{v}")).collect(),
            &self.raises,
        );
        for (source, counters) in ["engine", "fifo"].iter().zip(&self.edges) {
            copy(
                (0..BINS).map(|v| format!("edge_{source}_v{v}")).collect(),
                counters,
            );
        }
        Snapshot { at_ns, values }
    }

    /// Status-thread report: cumulative counts plus rates since its preceding report.
    /// The previous values are atomic too; no observer lock is taken on any thread.
    #[must_use]
    pub fn status_json(&self, at_ns: u64) -> String {
        let current = self.snapshot(at_ns);
        let elapsed = at_ns.saturating_sub(self.status_at.swap(at_ns, Ordering::Relaxed));
        let rates = Snapshot {
            at_ns: elapsed,
            values: current
                .values
                .iter()
                .zip(&self.status_previous)
                .map(|((key, v), old)| {
                    (
                        key.clone(),
                        v.saturating_sub(old.swap(*v, Ordering::Relaxed)),
                    )
                })
                .collect(),
        };
        current.json_with_rates(&rates)
    }
}

/// Cumulative or subtracted counts over explicitly named monotonic boundaries.
pub struct Snapshot {
    /// Census-relative endpoint (or elapsed time after subtraction).
    pub at_ns: u64,
    values: BTreeMap<String, u64>,
}

impl Snapshot {
    /// A missing sparse vector bin is zero.
    #[must_use]
    pub fn get(&self, name: &str) -> u64 {
        self.values.get(name).copied().unwrap_or(0)
    }

    /// Counts in `(before, self]`; never divide cumulative boot totals by workload frames.
    #[must_use]
    pub fn since(&self, before: &Self) -> Self {
        Self {
            at_ns: self.at_ns.saturating_sub(before.at_ns),
            values: self
                .values
                .iter()
                .map(|(key, value)| (key.clone(), value.saturating_sub(before.get(key))))
                .collect(),
        }
    }

    /// Rate over this snapshot's explicit elapsed window.
    #[must_use]
    pub fn per_second(&self, name: &str) -> Option<f64> {
        (self.at_ns != 0).then(|| self.get(name) as f64 * 1e9 / self.at_ns as f64)
    }

    /// Rate over externally observed presents in precisely this window (e.g. ETW).
    #[must_use]
    pub fn per_frame(&self, name: &str, frames: u64) -> Option<f64> {
        (frames != 0).then(|| self.get(name) as f64 / frames as f64)
    }

    fn json_with_rates(&self, rates: &Self) -> String {
        let visible = |key: &str, value: u64| {
            value != 0
                || !(key.starts_with("raise_v")
                    || key.starts_with("edge_engine_v")
                    || key.starts_with("edge_fifo_v"))
        };
        let counts = self
            .values
            .iter()
            .filter(|(key, value)| visible(key, **value))
            .map(|(k, v)| format!("\"{k}\":{v}"))
            .collect::<Vec<_>>()
            .join(",");
        let per_second = self
            .values
            .iter()
            .filter(|(key, value)| visible(key, **value))
            .map(|(k, _)| format!("\"{k}\":{:.6}", rates.per_second(k).unwrap_or(0.0)))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            concat!(
                "{{\"at_ns\":{},\"window_ns\":{},\"counts\":{{{}}},",
                "\"per_s\":{{{}}}}}"
            ),
            self.at_ns, rates.at_ns, counts, per_second,
        )
    }
}
