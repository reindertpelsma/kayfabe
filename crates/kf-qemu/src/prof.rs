//! ★ **w827 attribution instruments — OFF unless `KF3_PROF=1`.**
//!
//! Where the guest's run time goes, measured from inside the device: a per-offset census of the
//! BAR0 writes the vCPUs delivered (count and handler time), the register drainer's wake latency
//! and park outcomes, per-RPC service/hold latency, and per-thread busy time. Printed as
//! `kf3: PROF …` lines on the drainer's heartbeat (never a vCPU).
//!
//! ⊘ Never a decision input. ⊘ Off by default: every hook is one relaxed load of [`on`] when the
//! variable is unset. When on, a vCPU write pays two clock reads and a few relaxed atomics.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

static ON: AtomicBool = AtomicBool::new(false);
static T0: OnceLock<Instant> = OnceLock::new();

/// ★ Q7: bytes the Translated rings READ through a CPU view of vidmem (a vidmem GPFIFO or
/// pushbuffer), the calls, and their time — counted always (three relaxed adds per read).
pub static VIEW_READ_BYTES: AtomicU64 = AtomicU64::new(0);
/// Reads through a vidmem CPU view.
pub static VIEW_READS: AtomicU64 = AtomicU64::new(0);
/// Their time, ns (only when [`on`]).
pub static VIEW_READ_NS: AtomicU64 = AtomicU64::new(0);
/// Bytes the Translated rings read from guest RAM (the same pump, for comparison).
pub static RAM_READ_BYTES: AtomicU64 = AtomicU64::new(0);

/// The window-mark register (`CPU_INTR_LEAF(7)`, W1C) and value (`"KF3P"`).
pub const MARK_OFF: u64 = 0x00B8_101C;
/// See [`MARK_OFF`].
pub const MARK_VALUE: u64 = 0x4B46_3350;

/// Read `KF3_PROF` once (realize).
pub fn init() {
    let on = std::env::var("KF3_PROF").is_ok_and(|v| v == "1" || v == "on");
    let _ = T0.get_or_init(Instant::now);
    ON.store(on, Ordering::Relaxed);
    if on {
        eprintln!("kf3: PROF armed (KF3_PROF=1) — attribution counters on; timing costs two clock reads per trap");
    }
}

/// Whether the instruments are on.
#[inline]
pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Nanoseconds since realize.
#[inline]
pub fn now_ns() -> u64 {
    T0.get().map_or(0, |t| u64::try_from(t.elapsed().as_nanos()).unwrap_or(u64::MAX))
}

/// A log-linear latency histogram (4 sub-buckets per octave of nanoseconds), lock-free.
pub struct Hist {
    b: [AtomicU64; 256],
    n: AtomicU64,
    sum: AtomicU64,
    max: AtomicU64,
}

impl Default for Hist {
    fn default() -> Self {
        Hist {
            b: core::array::from_fn(|_| AtomicU64::new(0)),
            n: AtomicU64::new(0),
            sum: AtomicU64::new(0),
            max: AtomicU64::new(0),
        }
    }
}

fn bucket(ns: u64) -> usize {
    if ns < 4 {
        return ns as usize;
    }
    let lg = 63 - ns.leading_zeros() as usize; // >= 2
    let sub = ((ns >> (lg - 2)) & 3) as usize;
    (lg * 4 + sub).min(255)
}

fn bucket_hi(i: usize) -> u64 {
    // 0..=3 are exact; 4..=7 are never produced by `bucket` (4 ns is octave 2 → index 8).
    if i < 8 {
        return i.min(3) as u64;
    }
    let lg = i / 4;
    let sub = (i % 4) as u64;
    ((4 + sub + 1) << (lg - 2)).saturating_sub(1)
}

impl Hist {
    /// Record one sample.
    pub fn add(&self, ns: u64) {
        self.b[bucket(ns)].fetch_add(1, Ordering::Relaxed);
        self.n.fetch_add(1, Ordering::Relaxed);
        self.sum.fetch_add(ns, Ordering::Relaxed);
        self.max.fetch_max(ns, Ordering::Relaxed);
    }

    /// Samples.
    pub fn count(&self) -> u64 {
        self.n.load(Ordering::Relaxed)
    }

    /// Sum, ns.
    pub fn sum(&self) -> u64 {
        self.sum.load(Ordering::Relaxed)
    }

    fn pct(&self, p: f64) -> u64 {
        let n = self.count();
        if n == 0 {
            return 0;
        }
        let want = ((n as f64) * p).ceil().max(1.0) as u64;
        let mut acc = 0;
        for (i, b) in self.b.iter().enumerate() {
            acc += b.load(Ordering::Relaxed);
            if acc >= want {
                return bucket_hi(i);
            }
        }
        self.max.load(Ordering::Relaxed)
    }

    /// `n= sum_ms= avg_us= p50_us= p90_us= p99_us= max_us=` (percentiles are bucket upper bounds, ≤25% high).
    pub fn line(&self) -> String {
        let n = self.count();
        let us = |ns: u64| ns as f64 / 1000.0;
        format!(
            "n={n} sum_ms={:.1} avg_us={:.1} p50_us={:.1} p90_us={:.1} p99_us={:.1} max_us={:.1}",
            self.sum() as f64 / 1e6,
            us(self.sum().checked_div(n).unwrap_or(0)),
            us(self.pct(0.50)),
            us(self.pct(0.90)),
            us(self.pct(0.99)),
            us(self.max.load(Ordering::Relaxed)),
        )
    }
}

const SLOTS: usize = 4096;

/// Per-BAR0-offset census: count and vCPU handler time. Open addressing, lock-free; a full table
/// counts the overflow.
pub struct OffTable {
    key: Box<[AtomicU64]>,
    n: Box<[AtomicU64]>,
    ns: Box<[AtomicU64]>,
    overflow: AtomicU64,
}

impl Default for OffTable {
    fn default() -> Self {
        let mk = || (0..SLOTS).map(|_| AtomicU64::new(0)).collect::<Vec<_>>().into_boxed_slice();
        OffTable { key: mk(), n: mk(), ns: mk(), overflow: AtomicU64::new(0) }
    }
}

impl OffTable {
    /// Count one write at `off` that took `ns` in the handler.
    pub fn add(&self, off: u64, ns: u64) {
        let k = off + 1;
        let mut i = ((off >> 2).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 52) as usize % SLOTS;
        for _ in 0..SLOTS {
            let cur = self.key[i].load(Ordering::Relaxed);
            if cur == k
                || (cur == 0
                    && match self.key[i].compare_exchange(0, k, Ordering::Relaxed, Ordering::Relaxed) {
                        Ok(_) => true,
                        Err(now) => now == k,
                    })
            {
                self.n[i].fetch_add(1, Ordering::Relaxed);
                self.ns[i].fetch_add(ns, Ordering::Relaxed);
                return;
            }
            i = (i + 1) % SLOTS;
        }
        self.overflow.fetch_add(1, Ordering::Relaxed);
    }

    /// `(offset, count, handler ns)`, by count descending.
    pub fn rows(&self) -> Vec<(u64, u64, u64)> {
        let mut v: Vec<_> = (0..SLOTS)
            .filter_map(|i| {
                let k = self.key[i].load(Ordering::Relaxed);
                (k != 0).then(|| (k - 1, self.n[i].load(Ordering::Relaxed), self.ns[i].load(Ordering::Relaxed)))
            })
            .collect();
        v.sort_by_key(|r| std::cmp::Reverse(r.1));
        v
    }
}

/// One thread's wait accounting: time inside its poller wait vs outside it.
#[derive(Default)]
pub struct Busy {
    /// ns outside the wait (working).
    pub busy_ns: AtomicU64,
    /// ns inside the wait.
    pub wait_ns: AtomicU64,
    /// The longest single stretch outside a wait.
    pub max_busy_ns: AtomicU64,
    /// Waits entered.
    pub waits: AtomicU64,
    /// Waits that ended by TIMEOUT (no fd ready).
    pub timeouts: AtomicU64,
    /// Timeouts after which the very next pass found work — a lost or late wake.
    pub timeouts_with_work: AtomicU64,
}

impl Busy {
    /// Account a busy stretch that ended now.
    pub fn busy(&self, ns: u64) {
        self.busy_ns.fetch_add(ns, Ordering::Relaxed);
        self.max_busy_ns.fetch_max(ns, Ordering::Relaxed);
    }

    /// Account a wait.
    pub fn waited(&self, ns: u64, timed_out: bool) {
        self.wait_ns.fetch_add(ns, Ordering::Relaxed);
        self.waits.fetch_add(1, Ordering::Relaxed);
        if timed_out {
            self.timeouts.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// `busy_ms= wait_ms= busy_pct= max_busy_us= waits= timeouts= timeouts_with_work=`.
    pub fn line(&self) -> String {
        let o = Ordering::Relaxed;
        let b = self.busy_ns.load(o);
        let w = self.wait_ns.load(o);
        format!(
            "busy_ms={:.1} wait_ms={:.1} busy_pct={:.2} max_busy_us={:.1} waits={} timeouts={} timeouts_with_work={}",
            b as f64 / 1e6,
            w as f64 / 1e6,
            if b + w == 0 { 0.0 } else { 100.0 * b as f64 / (b + w) as f64 },
            self.max_busy_ns.load(o) as f64 / 1000.0,
            self.waits.load(o),
            self.timeouts.load(o),
            self.timeouts_with_work.load(o),
        )
    }
}

/// The device's instruments.
#[derive(Default)]
pub struct Prof {
    /// BAR0 writes by offset.
    pub bar0: OffTable,
    /// vCPU handler time of every BAR0 write.
    pub bar0_handler: Hist,
    /// `ns` when a vCPU last signalled the drainer's eventfd (0 = consumed).
    pub drainer_signal_ns: AtomicU64,
    /// Signal → drainer back from its wait.
    pub drainer_wake: Hist,
    /// The drainer's accounting.
    pub drainer: Busy,
    /// The VA manager's accounting.
    pub vamgr: Busy,
    /// `ns` of the most recent GSP command-queue-head write (the RPC doorbell), stamped on the vCPU.
    pub qhead_ns: AtomicU64,
    /// Queue-head trap → the drainer starts applying it.
    pub rpc_trap_to_apply: Hist,
    /// Apply → reply published (commands serviced, not held).
    pub rpc_service: Hist,
    /// Queue-head trap → reply published, for replies posted at once.
    pub rpc_immediate: Hist,
    /// Queue-head trap → held reply released.
    pub rpc_held: Hist,
    /// Held replies released by `release_settled` (after a VA/act wake) vs inline.
    pub held_released_late: AtomicU64,
    /// Commands serviced per queue-head write > 1 (coalesced doorbells).
    pub rpc_commands: AtomicU64,
    /// Queue-head writes applied.
    pub rpc_doorbells: AtomicU64,
    /// Drainer applies of non-queue-head registers.
    pub other_applies: Hist,
    /// Publish (shadow re-store) time per apply.
    pub publish: Hist,
    /// Invalidate trigger armed (vCPU) → published idle.
    pub inval_trap_ns: AtomicU64,
    /// Heartbeat prints since realize.
    pub beats: AtomicU64,
    /// ★ Client window marks: a guest write of [`MARK_VALUE`] to [`MARK_OFF`] (a write-1-to-clear
    /// leaf of the CPU interrupt tree no vector of ours lives in) bumps this; the drainer prints a
    /// full snapshot tagged with it, so a run can be diffed over exactly the client's window.
    pub marks: AtomicU64,
}

impl Prof {
    /// The `kf3: PROF …` lines.
    pub fn lines(&self, names: &dyn Fn(u64) -> &'static str) -> Vec<String> {
        let mut out = Vec::new();
        let t = now_ns() as f64 / 1e9;
        out.push(format!("kf3: PROF t={t:.3}s bar0_handler {}", self.bar0_handler.line()));
        let rows = self.bar0.rows();
        let total: u64 = rows.iter().map(|r| r.1).sum();
        out.push(format!(
            "kf3: PROF t={t:.3}s bar0_offsets distinct={} total={} overflow={}",
            rows.len(),
            total,
            self.bar0.overflow.load(Ordering::Relaxed)
        ));
        for (off, n, ns) in rows.iter().take(40) {
            out.push(format!(
                "kf3: PROF bar0 off={off:#08x} n={n} handler_avg_ns={} name={}",
                ns / (*n).max(1),
                names(*off)
            ));
        }
        out.push(format!("kf3: PROF drainer_wake {}", self.drainer_wake.line()));
        out.push(format!("kf3: PROF drainer {}", self.drainer.line()));
        out.push(format!("kf3: PROF vamgr {}", self.vamgr.line()));
        out.push(format!(
            "kf3: PROF rpc doorbells={} commands={} held_released_late={}",
            self.rpc_doorbells.load(Ordering::Relaxed),
            self.rpc_commands.load(Ordering::Relaxed),
            self.held_released_late.load(Ordering::Relaxed)
        ));
        out.push(format!("kf3: PROF rpc_trap_to_apply {}", self.rpc_trap_to_apply.line()));
        out.push(format!("kf3: PROF rpc_service {}", self.rpc_service.line()));
        out.push(format!("kf3: PROF rpc_immediate {}", self.rpc_immediate.line()));
        out.push(format!("kf3: PROF rpc_held {}", self.rpc_held.line()));
        out.push(format!("kf3: PROF other_applies {}", self.other_applies.line()));
        out.push(format!("kf3: PROF publish {}", self.publish.line()));
        out.push(format!(
            "kf3: PROF vidmem_view_reads n={} bytes={} ms={:.3} guest_ram_read_bytes={}",
            VIEW_READS.load(Ordering::Relaxed),
            VIEW_READ_BYTES.load(Ordering::Relaxed),
            VIEW_READ_NS.load(Ordering::Relaxed) as f64 / 1e6,
            RAM_READ_BYTES.load(Ordering::Relaxed),
        ));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_are_monotone_and_bound_their_samples() {
        let mut last = 0;
        for i in 0..200 {
            let hi = bucket_hi(i);
            assert!(hi >= last, "bucket {i}");
            last = hi;
        }
        for ns in [0u64, 1, 3, 4, 5, 7, 8, 1000, 1023, 1024, 123_456, 9_999_999] {
            let b = bucket(ns);
            assert!(bucket_hi(b) >= ns, "{ns} in bucket {b} (hi {})", bucket_hi(b));
            assert!(b == 0 || bucket_hi(b - 1) < ns, "{ns} not in the lowest bucket that holds it");
        }
    }

    #[test]
    fn the_offset_table_counts_per_offset() {
        let t = OffTable::default();
        for _ in 0..5 {
            t.add(0x110c00, 10);
        }
        t.add(0x0, 3);
        let r = t.rows();
        assert_eq!(r[0], (0x110c00, 5, 50));
        assert_eq!(r[1], (0x0, 1, 3));
    }
}
