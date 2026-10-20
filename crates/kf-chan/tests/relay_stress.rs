//! 2026-10-20 -- **Real-thread chaos stress of the USER-ring relay and its worker** (GPU-free).
//!
//! WHAT RUNS FOR REAL (production code, unchanged): `kf_chan::worker::run_with_relay` (the worker loop, `LoopPolicy`, the spin, the
//! park, the backstop), `kf_core::Plane::worker_pass` (claim / `ActAgain` / republish), the real token words, rung bitmap and wake
//! word of `kf-trap` (doorbells go through `Plane::trap_write(Class::Doorbell, ..)`), `kf_chan::user_turn::Relay` (the per-entry
//! turn), `Lifecycle`, `Fetch`, `Marks`, `Scan`, `user_loop::{may_retry, look_needed, found, Wait, Stats}`, a real epoll `Poller`
//! and real eventfds (`Notifier`) for the worker doorbell and the fake engine's non-stall event.
//!
//! WHAT IS A TRANSLITERATION (CODE READ, not the production function): the glue that lives in `kf-qemu` and cannot be built without a
//! GPU: `userrelay::Cell` (copied verbatim), `ChanPlane::user_ring_serve / user_ring_arm / user_ring_waiting / user_ring_peek /
//! user_ring_peek_one` (`crates/kf-qemu/src/userpump.rs`, mirrored arm for arm; `a_source_pin_...` below fails when the pinned
//! lines drift) and the segment machinery of the pump (`tests/lifecycle_support`, the existing stand-in). The fakes are the PLANE
//! (guest ring, GP_PUT, USERD GP_GET write-back, the pt / record / storage locks), the GPU (a PBDMA thread popping the private
//! ring) and the engine interrupt.
//!
//! Seeds are deterministic for the SCENARIO (clients, roles, rates, chaos probabilities, per-thread PRNG streams); the OS still
//! schedules the threads, so a seed biases the schedule and is re-run many times, it does not replay it.
//!
//! INVARIANTS: I1 no entry lost / duplicated / reordered (the fake PBDMA checks the sequence in order); I2 bounded wait: the
//! response of the relay after the hardware finished (guest `GP_GET` covering the batch) within `RS_BOUND_MS` of wall time, no client
//! starved; I3 a full guest ring releases; I4 loop counters consistent and no lost wake (`backstop_found_work == 0`,
//! `timeouts_with_work == 0`); I5 no panic, no poisoned lock, no refusal.
//!
//! Default suite: a short smoke (a few seeds, ~1 s each). Long runs: `RS_FROM=0 RS_TO=2000 cargo test --release -p kf-chan
//! --test relay_stress -- --ignored --nocapture many_seeds`.
#![allow(
    dead_code,
    missing_docs,
    clippy::too_many_lines,
    clippy::type_complexity,
    clippy::collapsible_if,
    clippy::needless_range_loop
)]

#[path = "lifecycle_support/mod.rs"]
mod support;

use kf_chan::completions::Completions;
use kf_chan::user_gate::Marks;
use kf_chan::user_lifecycle::Lifecycle;
use kf_chan::user_loop::{
    self as ul, Look, LookIn, Need, Peek, Stats as RelayStats, Wait, wait_of,
};
use kf_chan::user_scan::Scan;
use kf_chan::worker::{self, Inflight, RelayLoop, WorkerStats};
use kf_core::{HostOps, HostSlice, Owner, Plane, Translatable, VmCaps, Vmm};
use kf_linux_raw::{Notifier, Poller};
use kf_trap::{Action, Class, Route};
use std::cell::RefCell;
use std::collections::{BinaryHeap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use support::{Env, Ev, Glue, Pub, Relay};

// ------------------------------------------------------------------------------------------------
// constants

/// The guest ring (GPFIFO) entries and the private ring entries of the Windows relay.
const GUEST_N: u32 = 32768;
const PRIV_N: u32 = 512;
const DB_OFF: u32 = 0x00BB_0090;

// ------------------------------------------------------------------------------------------------
// PRNG and the chaos hook

#[derive(Clone)]
struct Rng(u64);
impl Rng {
    fn new(seed: u64, stream: u64) -> Self {
        let mut r = Rng(seed ^ stream.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03);
        for _ in 0..4 {
            r.next();
        }
        r
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.below(hi - lo + 1)
    }
    /// `true` with probability `num / 1024`.
    fn p(&mut self, num: u32) -> bool {
        (self.next() & 1023) < u64::from(num)
    }
}

/// The schedule-chaos profile of one thread: per 1024 at each hook point.
#[derive(Clone, Copy, Debug)]
struct ChaosCfg {
    yield_p: u32,
    spin_p: u32,
    sleep0_p: u32,
    nap_p: u32,
    nap_us_max: u32,
}
impl ChaosCfg {
    const OFF: Self = Self {
        yield_p: 0,
        spin_p: 0,
        sleep0_p: 0,
        nap_p: 0,
        nap_us_max: 0,
    };
}
struct Chaos {
    rng: Rng,
    cfg: ChaosCfg,
}
thread_local! {
    static CHAOS: RefCell<Option<Chaos>> = const { RefCell::new(None) };
}
fn chaos_install(seed: u64, stream: u64, cfg: ChaosCfg) {
    CHAOS.with(|c| {
        *c.borrow_mut() = Some(Chaos {
            rng: Rng::new(seed, 0xC4A05 + stream),
            cfg,
        })
    });
}
fn chaos_remove() {
    CHAOS.with(|c| *c.borrow_mut() = None);
}
/// One chaos point: a seeded yield / spin / `sleep(0)` / (rare) short nap.
fn chaos() {
    CHAOS.with(|c| {
        let mut c = c.borrow_mut();
        let Some(ch) = c.as_mut() else { return };
        let cfg = ch.cfg;
        if ch.rng.p(cfg.yield_p) {
            std::thread::yield_now();
        }
        if ch.rng.p(cfg.spin_p) {
            for _ in 0..ch.rng.range(10, 400) {
                std::hint::spin_loop();
            }
        }
        if ch.rng.p(cfg.sleep0_p) {
            std::thread::sleep(Duration::ZERO);
        }
        if ch.rng.p(cfg.nap_p) && cfg.nap_us_max > 0 {
            std::thread::sleep(Duration::from_micros(
                ch.rng.range(1, u64::from(cfg.nap_us_max)),
            ));
        }
    });
}
thread_local! {
    static AFTER_WAITS: RefCell<Option<Arc<AtomicU64>>> = const { RefCell::new(None) };
}
/// The production hook points (`kf_trap::chaos`, feature `chaos-hooks`). `WorkerAfterWait` also counts ended park waits (I6).
fn hook(p: kf_trap::chaos::Point) {
    if p == kf_trap::chaos::Point::WorkerAfterWait {
        AFTER_WAITS.with(|a| {
            if let Some(a) = a.borrow().as_ref() {
                a.fetch_add(1, Ordering::AcqRel);
            }
        });
    }
    chaos();
}

// ------------------------------------------------------------------------------------------------
// histogram

#[derive(Clone, Debug)]
struct Hist {
    b: [u64; 40],
    n: u64,
    max_ns: u64,
}
impl Default for Hist {
    fn default() -> Self {
        Self {
            b: [0; 40],
            n: 0,
            max_ns: 0,
        }
    }
}
impl Hist {
    fn add(&mut self, ns: u64) {
        let us = (ns / 1000).max(1);
        self.b[(63 - us.leading_zeros()) as usize] += 1;
        self.n += 1;
        self.max_ns = self.max_ns.max(ns);
    }
    fn merge(&mut self, o: &Hist) {
        for i in 0..40 {
            self.b[i] += o.b[i];
        }
        self.n += o.n;
        self.max_ns = self.max_ns.max(o.max_ns);
    }
    fn p(&self, q: f64) -> u64 {
        if self.n == 0 {
            return 0;
        }
        let want = (self.n as f64 * q).ceil() as u64;
        let mut acc = 0;
        for i in 0..40 {
            acc += self.b[i];
            if acc >= want {
                return (1u64 << (i + 1)).min(self.max_ns / 1000 + 1);
            }
        }
        self.max_ns / 1000
    }
    fn line(&self) -> String {
        let used: Vec<String> = self
            .b
            .iter()
            .enumerate()
            .filter(|(_, c)| **c > 0)
            .map(|(i, c)| format!("{}us:{c}", 1u64 << i))
            .collect();
        format!(
            "n={} p50<={}us p99<={}us max={:.2}ms [{}]",
            self.n,
            self.p(0.5),
            self.p(0.99),
            self.max_ns as f64 / 1e6,
            used.join(" ")
        )
    }
}

// ------------------------------------------------------------------------------------------------
// userrelay::Cell (kf-qemu/src/userrelay.rs, verbatim apart from visibility)

struct Cell<T> {
    idx: u32,
    retry: AtomicBool,
    canceled: AtomicBool,
    waiting: AtomicBool,
    carried: AtomicBool,
    contended: AtomicU32,
    inner: Mutex<T>,
}
impl<T> Cell<T> {
    fn new(idx: u32, value: T) -> Self {
        Self {
            idx,
            retry: AtomicBool::new(false),
            canceled: AtomicBool::new(false),
            waiting: AtomicBool::new(false),
            carried: AtomicBool::new(false),
            contended: AtomicU32::new(0),
            inner: Mutex::new(value),
        }
    }
    fn lock(&self) -> std::sync::LockResult<std::sync::MutexGuard<'_, T>> {
        self.inner.lock()
    }
    fn try_lock(&self) -> std::sync::TryLockResult<std::sync::MutexGuard<'_, T>> {
        self.inner.try_lock()
    }
    fn enter(&self) -> std::sync::TryLockResult<std::sync::MutexGuard<'_, T>> {
        if !self.request() {
            return Err(std::sync::TryLockError::WouldBlock);
        }
        self.inner.try_lock().inspect_err(|e| {
            if matches!(e, std::sync::TryLockError::Poisoned(_)) {
                self.retry.store(false, Ordering::Release);
            }
        })
    }
    fn request(&self) -> bool {
        if self.canceled.load(Ordering::Acquire) {
            return false;
        }
        self.retry.store(true, Ordering::Release);
        if self.canceled.load(Ordering::Acquire) {
            self.retry.store(false, Ordering::Release);
            return false;
        }
        true
    }
    fn pending(&self) -> bool {
        !self.is_canceled() && self.retry.load(Ordering::Acquire)
    }
    fn is_canceled(&self) -> bool {
        self.canceled.load(Ordering::Acquire)
    }
    fn contended_run(&self) -> u32 {
        self.contended.load(Ordering::Acquire)
    }
    fn contended_note(&self) {
        let _ = self
            .contended
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                Some(n.saturating_add(1))
            });
    }
    fn contended_clear(&self) {
        self.contended.store(0, Ordering::Release);
    }
    fn is_waiting(&self) -> bool {
        !self.is_canceled() && self.waiting.load(Ordering::Acquire)
    }
    fn set_waiting(&self, waiting: bool) {
        self.waiting.store(waiting, Ordering::Release);
    }
    fn carried(&self) -> bool {
        self.carried.load(Ordering::Acquire)
    }
    fn set_carried(&self, carried: bool) {
        self.carried.store(carried, Ordering::Release);
    }
    fn cancel(&self) {
        self.canceled.store(true, Ordering::Release);
        self.retry.store(false, Ordering::Release);
        self.waiting.store(false, Ordering::Release);
    }
    fn wake_if(&self, active: impl FnOnce(&T) -> bool, ring: impl FnOnce(u32)) {
        if let Ok(r) = self.inner.try_lock() {
            if !self.canceled.load(Ordering::Acquire) && active(&r) {
                ring(self.idx);
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------
// scenario

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    /// Big batches, never waits for completion (only for a full ring): keeps its relay hot.
    Hot,
    /// 1..3 entries, waits for the completion, thinks: the cold client of the baton.
    Cold,
    /// Writes up to the end of the guest ring (32767 entries), rings, waits for half a ring of drain.
    Filler,
    /// A random mix of the above per iteration.
    Mixed,
}

#[derive(Clone, Copy, Debug)]
struct GpuCfg {
    /// Entries popped per client per visit (max).
    burst: u32,
    /// Per-1024 chance per visit of a stall of up to `stall_us_max`.
    stall_p: u32,
    stall_us_max: u32,
    /// Cost per popped entry (us, spin) -- 0 = free.
    cost_us: u32,
    /// Extra cost of a marker / barrier (a SYNC_WAIT acquire: the semaphore release arrives late).
    barrier_us_max: u32,
    /// The engine interrupt is delayed by up to this (us), per marker, independently (so deliveries reorder).
    irq_jitter_us: u32,
}

#[derive(Clone, Copy, Debug)]
struct HolderCfg {
    on: bool,
    /// Longest hold of the record / pt lock (us); the storage lock is held at most 1/10 of it.
    hold_us_max: u32,
    /// Per 1024: the holder rings the token after releasing the record (an act thread does; pt / storage holders do not).
    rings_after_p: u32,
    gap_us_max: u32,
}

#[derive(Clone, Debug)]
struct Scenario {
    seed: u64,
    roles: Vec<Role>,
    workers: usize,
    always_scan: bool,
    /// Per 1024 of an entry carrying SYNC_WAIT.
    sync_p: u32,
    gpu: GpuCfg,
    holder: HolderCfg,
    /// Per 1024 of each Io site reporting `Busy` (a retry).
    busy_p: u32,
    chaos: ChaosCfg,
    secs: f64,
    /// Per-1024 knobs of the doorbell styles.
    style_dup_p: u32,
    style_late_p: u32,
    style_skip_p: u32,
    /// SYNTHETIC hot tokens (after the real clients): a token whose every serve makes progress and rings itself again (a relay with
    /// unbounded input and credit). They are the "hot neighbours" of the baton, without depending on how fast the fake GPU is.
    synth_hot: usize,
    /// Cost of one turn / one synthetic serve (us of spin): the real pump's guarded accessors take tens of us (`RS_TURN_US` overrides).
    turn_us: u64,
}

impl Scenario {
    fn from_seed(seed: u64, secs: f64) -> Self {
        let mut r = Rng::new(seed, 1);
        let n = r.range(1, 8) as usize;
        let mut roles = Vec::new();
        for _ in 0..n {
            roles.push(match r.below(8) {
                0..=2 => Role::Hot,
                3..=4 => Role::Cold,
                5 => Role::Filler,
                _ => Role::Mixed,
            });
        }
        let gpu = match r.below(5) {
            0 => GpuCfg {
                burst: 4096,
                stall_p: 0,
                stall_us_max: 0,
                cost_us: 0,
                barrier_us_max: 0,
                irq_jitter_us: 0,
            },
            1 => GpuCfg {
                burst: 64,
                stall_p: 40,
                stall_us_max: 3000,
                cost_us: 0,
                barrier_us_max: 200,
                irq_jitter_us: 300,
            },
            2 => GpuCfg {
                burst: 16,
                stall_p: 8,
                stall_us_max: 30000,
                cost_us: 2,
                barrier_us_max: 1000,
                irq_jitter_us: 2000,
            },
            3 => GpuCfg {
                burst: 512,
                stall_p: 100,
                stall_us_max: 800,
                cost_us: 0,
                barrier_us_max: 50,
                irq_jitter_us: 100,
            },
            _ => GpuCfg {
                burst: 1,
                stall_p: 0,
                stall_us_max: 0,
                cost_us: 20,
                barrier_us_max: 100,
                irq_jitter_us: 500,
            },
        };
        let holder = if r.p(512) {
            HolderCfg {
                on: true,
                hold_us_max: [200, 2000, 20000][r.below(3) as usize],
                rings_after_p: [0, 512, 1024][r.below(3) as usize],
                gap_us_max: [100, 1000, 10000][r.below(3) as usize],
            }
        } else {
            HolderCfg {
                on: false,
                hold_us_max: 0,
                rings_after_p: 0,
                gap_us_max: 0,
            }
        };
        let chaos = match r.below(4) {
            0 => ChaosCfg::OFF,
            1 => ChaosCfg {
                yield_p: 64,
                spin_p: 16,
                sleep0_p: 16,
                nap_p: 0,
                nap_us_max: 0,
            },
            2 => ChaosCfg {
                yield_p: 256,
                spin_p: 128,
                sleep0_p: 128,
                nap_p: 4,
                nap_us_max: 200,
            },
            _ => ChaosCfg {
                yield_p: 512,
                spin_p: 256,
                sleep0_p: 256,
                nap_p: 16,
                nap_us_max: 1000,
            },
        };
        let mut sc = Scenario {
            seed,
            roles,
            workers: [1, 2, 2, 2, 3][r.below(5) as usize],
            always_scan: r.p(512),
            sync_p: [0, 4, 32, 256][r.below(4) as usize],
            gpu,
            holder,
            busy_p: [0, 0, 8, 64][r.below(4) as usize],
            chaos,
            secs,
            style_dup_p: 100,
            style_late_p: 100,
            style_skip_p: 100,
            synth_hot: 0,
            turn_us: 0,
        };
        sc.apply_overrides();
        sc
    }

    /// Minimisation knobs (env): `RS_CLIENTS` (keep the first n), `RS_WORKERS`, `RS_HOLDER=0|1`, `RS_CHAOS=0`, `RS_BUSY=0`, `RS_SCAN=0|1`,
    /// `RS_SYNC`, `RS_GPU=fast`.
    fn apply_overrides(&mut self) {
        if let Ok(v) = std::env::var("RS_CLIENTS") {
            self.roles.truncate(v.parse().unwrap_or(8));
        }
        if let Ok(v) = std::env::var("RS_ROLES") {
            self.roles = v
                .chars()
                .map(|c| match c {
                    'H' => Role::Hot,
                    'C' => Role::Cold,
                    'F' => Role::Filler,
                    _ => Role::Mixed,
                })
                .collect();
        }
        if std::env::var("RS_WORKERS").is_ok() {
            self.workers = env_u("RS_WORKERS", 2) as usize;
        }
        match std::env::var("RS_HOLDER").as_deref() {
            Ok("0") => self.holder.on = false,
            Ok("1") if !self.holder.on => {
                self.holder = HolderCfg {
                    on: true,
                    hold_us_max: 2000,
                    rings_after_p: 1024,
                    gap_us_max: 1000,
                };
            }
            _ => {}
        }
        if std::env::var("RS_CHAOS").as_deref() == Ok("0") {
            self.chaos = ChaosCfg::OFF;
        }
        if std::env::var("RS_BUSY").as_deref() == Ok("0") {
            self.busy_p = 0;
        }
        if let Ok(v) = std::env::var("RS_SCAN") {
            self.always_scan = v == "1";
        }
        if let Ok(v) = std::env::var("RS_SYNC") {
            self.sync_p = v.parse().unwrap_or(0);
        }
        if std::env::var("RS_GPU").as_deref() == Ok("fast") {
            self.gpu = GpuCfg {
                burst: 4096,
                stall_p: 0,
                stall_us_max: 0,
                cost_us: 0,
                barrier_us_max: 0,
                irq_jitter_us: 0,
            };
        }
    }

    /// The baton shape of the hardware report: two hot relays for two workers, a cold sync client, a free fast GPU, no lock holder.
    fn baton(seed: u64, secs: f64) -> Self {
        let mut s = Self::from_seed(seed, secs);
        s.roles = vec![Role::Hot; env_u("RS_HOT", 0) as usize];
        s.roles.push(Role::Cold);
        s.synth_hot = env_u("RS_SYNTH", 4) as usize;
        s.turn_us = 100;
        s.workers = env_u("RS_WORKERS", 2) as usize;
        s.gpu = GpuCfg {
            burst: 4096,
            stall_p: 0,
            stall_us_max: 0,
            cost_us: 0,
            barrier_us_max: 0,
            irq_jitter_us: 0,
        };
        s.holder = HolderCfg {
            on: false,
            hold_us_max: 0,
            rings_after_p: 0,
            gap_us_max: 0,
        };
        s.busy_p = 0;
        s.sync_p = 0;
        s.chaos = ChaosCfg::OFF;
        s
    }

    /// The exhausted-`try_lock` shape: a cold sync client whose record lock is held (not announced) beside hot relays that keep
    /// the two workers busy.
    fn contention(seed: u64, secs: f64) -> Self {
        let mut s = Self::baton(seed, secs);
        s.holder = HolderCfg {
            on: true,
            hold_us_max: 3000,
            rings_after_p: 0,
            gap_us_max: 300,
        };
        s
    }
}

// ------------------------------------------------------------------------------------------------
// shared state

/// The fake plane + fake GPU state of one client (one channel).
struct Client {
    idx: u32,
    ring_lo: Vec<AtomicU32>,
    ring_hi: Vec<AtomicU32>,
    gp_put: AtomicU32,
    /// The guest-visible USERD `GP_GET`, written back by the relay.
    gp_get: AtomicU32,
    /// The private ring: what the relay published and the PBDMA has not popped.
    pq: Mutex<VecDeque<Pub>>,
    /// The hardware words (the private USERD): pops (wraps at `PRIV_N`) and the tagged reference.
    hw_get: AtomicU32,
    hw_ref: AtomicU32,
    /// Guest entries the PBDMA executed (absolute) and when the last one was.
    exec_abs: AtomicU64,
    /// Relay counters.
    published: AtomicU64,
    /// The `lease.storage` lock (a try_lock in the peek, a lock in the turn).
    storage: Mutex<()>,
    /// Wall time (ns) of the last pop, for the response latency.
    last_pop_ns: AtomicU64,
    /// FORENSICS (I6): when the fake PBDMA last stored REFERENCE.
    ref_ns: AtomicU64,
}
impl Client {
    fn new(idx: u32) -> Self {
        Self {
            idx,
            ring_lo: (0..GUEST_N).map(|_| AtomicU32::new(0)).collect(),
            ring_hi: (0..GUEST_N).map(|_| AtomicU32::new(0)).collect(),
            gp_put: AtomicU32::new(0),
            gp_get: AtomicU32::new(0),
            pq: Mutex::new(VecDeque::new()),
            hw_get: AtomicU32::new(0),
            hw_ref: AtomicU32::new(0),
            exec_abs: AtomicU64::new(0),
            published: AtomicU64::new(0),
            storage: Mutex::new(()),
            last_pop_ns: AtomicU64::new(0),
            ref_ns: AtomicU64::new(0),
        }
    }
}

/// Entry `n` (0 = the software-object bind): a real C56F GP entry whose pushbuffer VA names `n`.
fn entry_raw(n: u64, sync: bool) -> u64 {
    if n == 0 {
        return support::ge(0, support::Kind::SetObj).raw();
    }
    let va = 0x1000 * ((n % (1 << 24)) + 1);
    kf_abi::submit::gp_entry(va, 8).expect("encodable") | if sync { support::SYNC } else { 0 }
}
fn entry_n(raw: u64) -> u64 {
    let raw = raw & !support::SYNC;
    match kf_chan::user_stream::inspect_entry(raw, 1 << 20) {
        Ok(kf_chan::user_stream::Source::Segment(d)) => d.gpu_va / 0x1000 - 1,
        _ => u64::MAX,
    }
}

struct Shared {
    t0: Instant,
    stop_gpu: AtomicBool,
    stop_workers: AtomicBool,
    abort: AtomicBool,
    violations: Mutex<Vec<String>>,
    soft: Mutex<Vec<String>>,
    /// Serves (calls of `run_translated`) -- the global progress clock of the "passes" bound.
    serves: AtomicU64,
    forges: AtomicU64,
    /// The first undelivered engine raise (ns, 0 = none) and the interrupt latency histogram.
    irq_pending_since: AtomicU64,
    /// Park waits that ended (hook `WorkerAfterWait`); `WorkerStats::parks - after_waits` = workers in a blocking wait now.
    after_waits: Arc<AtomicU64>,
    /// FORENSICS (I6): the last engine-event dispatch and the last irq raise (ns).
    last_dispatch_ns: AtomicU64,
    last_raise_ns: AtomicU64,
    irq_hist: Mutex<Hist>,
    irqs_raised: AtomicU64,
    irqs_dispatched: AtomicU64,
    bound_ns: u64,
    /// A wait over `bound_ns` is STARVATION only if the workers served more than this many tokens meanwhile (`RS_STARVE_SERVES`).
    starve_serves: u64,
    /// Global: `pt` lock of the fake plane.
    pt: Mutex<()>,
}
impl Shared {
    fn now(&self) -> u64 {
        self.t0.elapsed().as_nanos() as u64 + 1
    }
    /// A HARD violation (I1, I3 stuck, I5, a lost wake): stops the run.
    fn violation(&self, s: String) {
        let mut v = self.violations.lock().unwrap_or_else(|e| e.into_inner());
        if v.len() < 24 {
            v.push(format!("HARD {s}"));
        }
        self.abort.store(true, Ordering::Release);
    }
    /// A SOFT violation: bounded wait / safety-net findings of the contended-`try_lock` class (the case under repair elsewhere).
    /// Counted and reported; fails a run only under `RS_STRICT=1`.
    fn soft(&self, s: String) {
        let mut v = self.soft.lock().unwrap_or_else(|e| e.into_inner());
        if v.len() < 24 {
            v.push(format!("SOFT {s}"));
        }
    }
}

// ------------------------------------------------------------------------------------------------
// the relay's environment (the fake plane)

struct ClientEnv<'a> {
    c: &'a Client,
    sh: &'a Shared,
    rng: Rng,
    busy_p: u32,
    turn_us: u64,
}
impl ClientEnv<'_> {
    fn busy(&mut self) -> bool {
        self.busy_p > 0 && self.rng.p(self.busy_p)
    }
}
impl Env for ClientEnv<'_> {
    fn src_entries(&self) -> u32 {
        GUEST_N
    }
    fn gp_put(&mut self) -> u32 {
        chaos();
        if self.turn_us > 0 {
            // production's guarded USERD / ring accessors cost far more than an atomic load
            sleep_or_spin(self.turn_us);
        }
        self.c.gp_put.load(Ordering::Acquire)
    }
    fn gp_dword(&mut self, slot: u32, hi: bool) -> u32 {
        let v = if hi { &self.c.ring_hi } else { &self.c.ring_lo };
        v[slot as usize].load(Ordering::Acquire)
    }
    fn registry_epoch(&mut self) -> u64 {
        std::sync::atomic::fence(Ordering::SeqCst);
        0
    }
    fn registry_live(&mut self) -> u32 {
        0
    }
    fn hw_get(&mut self) -> u32 {
        // production `hw_words`: `lease.storage.lock()` (blocking), then both words
        chaos();
        match self.c.storage.lock() {
            Ok(_g) => {
                chaos();
                self.c.hw_get.load(Ordering::Acquire)
            }
            Err(_) => {
                self.sh.violation("I5 storage lock poisoned".into());
                0
            }
        }
    }
    fn hw_reference(&mut self) -> u32 {
        self.c.hw_ref.load(Ordering::Acquire)
    }
    fn read_pb(&mut self, raw: u64) -> Vec<u32> {
        if raw & !support::SYNC == support::ge(0, support::Kind::SetObj).raw() {
            return support::ge(0, support::Kind::SetObj).words();
        }
        vec![
            kf_abi::submit::method_header_inc(0, 0x100, 1).expect("header"),
            entry_n(raw) as u32,
        ]
    }
    fn publish(&mut self, p: Pub) {
        chaos();
        if let Pub::Guest { tested, stored, .. } = p {
            if tested != stored {
                self.sh
                    .violation(format!("I7 tested {tested:#x} != stored {stored:#x}"));
            }
        }
        self.c.published.fetch_add(1, Ordering::Relaxed);
        self.c
            .pq
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(p);
    }
    fn write_guest_get(&mut self, v: u32) {
        chaos();
        self.c.gp_get.store(v, Ordering::Release);
    }
    fn write_guest_get_busy(&mut self) -> bool {
        // `userd.write` -> `AccessError::Busy` when the pt lock is busy
        self.sh.pt.try_lock().is_err() || self.busy()
    }
    fn busy_at(&mut self, _site: &'static str) -> bool {
        self.sh.pt.try_lock().is_err() || self.busy()
    }
    fn trigger_migrates(&mut self, _raw: u64) -> bool {
        false
    }
    fn executed(&mut self, _migrate: bool) {}
    fn event(&mut self, e: Ev) {
        if matches!(
            e,
            Ev::BeyondPut | Ev::PutBound | Ev::EntryRefused | Ev::ScanRefused
        ) {
            self.sh.violation(format!("I5 relay event {e:?}"));
        }
    }
}

// ------------------------------------------------------------------------------------------------
// the runtime: the mirrored `ChanPlane` glue

struct Rec {
    relay: Relay,
    env_rng_seed: u64,
    wait: Option<Wait>,
    dead: Option<String>,
    turns: u64,
    /// FORENSICS (I6): when the current wait was armed (ns).
    armed_ns: u64,
    /// FORENSICS (I6): the last look that held the record: (ns, words read, verdict).
    last_look: Option<(u64, Option<(u64, u64)>, Look)>,
}

struct Rt<'a> {
    plane: &'a Plane<'a>,
    sh: &'a Shared,
    clients: Vec<Arc<Client>>,
    cells: Vec<Arc<Cell<Rec>>>,
    /// `user_rings`: the map the peek locks.
    rings: Mutex<Vec<Arc<Cell<Rec>>>>,
    wake: &'a Notifier,
    stats: &'a RelayStats,
    busy_p: u32,
    /// Spin per turn (`RS_TURN_US`): the cost of the guarded accessors of the real pump.
    turn_us: u64,
    /// Tokens at or above this are synthetic hot tokens.
    n_real: u32,
    /// "run_translated calls" -- must equal `WorkerStats::served` (I4).
    serve_calls: AtomicU64,
}

impl Rt<'_> {
    fn ring_token(&self, idx: u32) -> bool {
        if self.plane.ring_internal(idx) {
            let _ = self.wake.signal();
        }
        true
    }

    /// MIRROR of `ChanPlane::user_ring_serve` (kf-qemu/src/userpump.rs, 2724).
    fn serve(&self, ht: u32) -> Option<bool> {
        let record = self.cells.get(ht as usize)?;
        let Ok(mut r) = record.enter() else {
            record.contended_note();
            return Some(false);
        };
        if r.dead.is_some() {
            record.cancel();
            return Some(false);
        }
        chaos(); // inside a turn holding the record lock
        let pt = match self.sh.pt.try_lock() {
            Ok(pt) => pt,
            Err(std::sync::TryLockError::WouldBlock) => {
                record.contended_note();
                return Some(false);
            }
            Err(std::sync::TryLockError::Poisoned(_)) => {
                record.cancel();
                self.sh.violation("I5 pt lock poisoned".into());
                return Some(false);
            }
        };
        chaos(); // holding record + pt
        if self.sh.stop_workers.load(Ordering::Acquire) {
            record.retry.store(false, Ordering::Release);
            return Some(false);
        }
        drop(pt);
        let _woke = record.retry.swap(false, Ordering::AcqRel);
        let client = &self.clients[ht as usize];
        let rec = &mut *r;
        let mut env = ClientEnv {
            c: client,
            sh: self.sh,
            rng: Rng::new(rec.env_rng_seed, rec.turns),
            busy_p: self.busy_p,
            turn_us: self.turn_us,
        };
        rec.turns += 1;
        // `Pump::turn` + `turn_body`: claim, steps until the turn is over
        rec.relay.claim();
        let mut steps = 0;
        while !rec.relay.idle() {
            rec.relay.step(&mut env);
            steps += 1;
            if steps > 4096 {
                self.sh
                    .violation("I5 a turn did not end within 4096 micro-steps".into());
                record.cancel();
                return Some(false);
            }
        }
        if let Some(why) = rec.relay.last_refused.take() {
            self.sh.violation(format!("I5 relay {ht} refused: {why}"));
            rec.dead = Some(why);
            record.cancel();
            return Some(false);
        }
        let retry = rec.relay.last_retry;
        if retry {
            rec.relay.core.need = Need::Contended;
        } else if rec.relay.core.again {
            rec.relay.core.need = Need::Again;
        }
        self.arm(record, rec);
        Some(!retry)
    }

    /// MIRROR of `ChanPlane::user_ring_arm`.
    fn arm(&self, record: &Cell<Rec>, r: &mut Rec) {
        let idx = record.idx;
        let need = r.relay.core.need;
        r.wait = wait_of(need);
        r.armed_ns = self.sh.now();
        record.set_waiting(r.wait.is_some());
        match need {
            Need::Again => {
                record.contended_clear();
                self.ring_token(idx);
            }
            Need::Contended => {
                record.contended_note();
                let _ = record.request();
                RelayStats::bump(&self.stats.contended);
            }
            Need::Idle | Need::Marker { .. } | Need::Popped { .. } => record.contended_clear(),
        }
        record.set_carried(r.wait.is_some_and(Wait::carried));
    }

    /// MIRROR of `ChanPlane::user_ring_peek_one` (2026-10-20 simple relay: the one decision `user_loop::look`).
    fn peek_one(&self, record: &Arc<Cell<Rec>>, why: Peek) -> bool {
        if record.is_canceled() {
            return false;
        }
        let demand = record.pending();
        if !ul::look_needed(why, demand, record.is_waiting()) {
            return false;
        }
        let Ok(mut r) = record.try_lock() else {
            let i = LookIn {
                demand,
                record_busy: true,
                // MUTANT (RS_MUT_BUSY_LOOK_IGNORES_WAIT=1): the pre-aad1e9ff4 call site, for the mutation check of I6
                wait: if env_u("RS_MUT_BUSY_LOOK_IGNORES_WAIT", 0) != 0 {
                    None
                } else {
                    record
                        .is_waiting()
                        .then_some(Wait::Marker { target: u64::MAX })
                },
                words: None,
                fetched: 0,
                get_follows_pops: false,
            };
            return matches!(ul::look(i), Look::Ring { .. }) && self.ring_token(record.idx);
        };
        if r.dead.is_some() {
            return false;
        }
        let r = &mut *r;
        let client = &self.clients[record.idx as usize];
        let words = r.wait.and_then(|_| match client.storage.try_lock() {
            Ok(_storage) => {
                let get = client.hw_get.load(Ordering::Acquire);
                let reference = client.hw_ref.load(Ordering::Acquire);
                r.relay
                    .core
                    .fetch
                    .peek(get)
                    .ok()
                    .map(|popped| (r.relay.core.marks.peek(reference), popped))
            }
            Err(_) => None,
        });
        let i = LookIn {
            demand,
            record_busy: false,
            wait: r.wait,
            words,
            fetched: r.relay.core.fetch.fetched(),
            get_follows_pops: kf_chan::user_turn::guest_get_follows_pops(),
        };
        let verdict = ul::look(i);
        r.last_look = Some((self.sh.now(), words, verdict));
        match verdict {
            Look::Leave => {
                if why == Peek::Event && r.wait.is_some() {
                    RelayStats::bump(&self.stats.event_without_progress);
                }
                false
            }
            Look::Ring {
                clear_wait,
                pops_only,
            } => {
                if pops_only {
                    RelayStats::bump(&self.stats.pop_writebacks);
                }
                if clear_wait {
                    r.wait = None;
                    record.set_waiting(false);
                    record.set_carried(false);
                }
                self.ring_token(record.idx)
            }
        }
    }
}

impl RelayLoop for Rt<'_> {
    /// MIRROR of `user_ring_waiting`.
    fn waiting(&self) -> bool {
        self.rings.try_lock().map_or(true, |rings| {
            rings.iter().any(|c| c.is_waiting() || c.pending())
        })
    }
    /// MIRROR of `user_ring_unsourced`.
    fn unsourced(&self) -> bool {
        self.rings.try_lock().map_or(true, |rings| {
            rings
                .iter()
                .any(|c| c.pending() || (c.is_waiting() && c.carried()))
        })
    }
    /// MIRROR of `user_ring_peek`.
    fn peek(&self, why: Peek) -> bool {
        let Ok(rings) = self.rings.lock() else {
            return false;
        };
        let mut rang = false;
        for record in rings.iter() {
            rang |= self.peek_one(record, why);
        }
        rang
    }
}

impl HostOps for Rt<'_> {
    fn ring_host(&self, _: u32) {
        self.sh.violation("I5 ring_host reached".into());
    }
    fn run_translated(&self, ht: u32, _: u64) -> bool {
        self.sh.serves.fetch_add(1, Ordering::Relaxed);
        self.serve_calls.fetch_add(1, Ordering::Relaxed);
        if ht >= self.n_real {
            // a synthetic hot relay: progress, and its own token rung again (`Need::Again` -> `ring_internal`)
            if self.turn_us > 0 {
                sleep_or_spin(self.turn_us);
            }
            self.ring_token(ht);
            return true;
        }
        self.serve(ht).unwrap_or(false)
    }
    fn run_emulated(&self, _: u32, _: u64) {
        self.sh.violation("I5 run_emulated reached".into());
    }
    fn apply_register(&self, _: u8, _: u32, _: u64, _: u8) {}
    fn operands_translatable(&self, _: u32, _: u64) -> Translatable {
        Translatable::Yes
    }
    fn forge_completion(&self, _: u32) {
        self.sh.forges.fetch_add(1, Ordering::Relaxed);
        self.sh.violation("I5 a completion was forged".into());
    }
    fn refuse_and_poison(&self, _: u32) {
        self.sh.violation("I5 refuse_and_poison".into());
    }
    fn fault_channel(&self, _: u32) {
        self.sh.violation("I5 fault_channel".into());
    }
    fn map_guest_slice(&self, _: HostSlice) {}
    fn teardown_step(&self, _: kf_core::Step) {}
}

/// The session's completion set: nothing is ever in flight (no host fence exists).
struct NoInflight;
impl Inflight for NoInflight {
    fn for_each_inflight(&self, _f: &mut dyn FnMut(u32)) {}
}

// ------------------------------------------------------------------------------------------------
// the fake GPU (PBDMA) and the engine interrupt

#[derive(PartialEq, Eq)]
struct Due(u64);
impl Ord for Due {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        o.0.cmp(&self.0)
    }
}
impl PartialOrd for Due {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

fn sleep_or_spin(us: u64) {
    if us == 0 {
        return;
    }
    if us < 30 {
        let t = Instant::now();
        while t.elapsed() < Duration::from_micros(us) {
            std::hint::spin_loop();
        }
    } else {
        std::thread::sleep(Duration::from_micros(us));
    }
}

fn gpu_thread(
    sh: &Shared,
    clients: &[Arc<Client>],
    cfg: GpuCfg,
    irqs: &Mutex<BinaryHeap<Due>>,
    seed: u64,
    ch: ChaosCfg,
) {
    chaos_install(seed, 900, ch);
    let mut rng = Rng::new(seed, 900);
    let mut popped_total = vec![0u64; clients.len()];
    let mut next_ready = vec![0u64; clients.len()];
    let mut start = 0usize;
    while !sh.stop_gpu.load(Ordering::Acquire) {
        let mut did = false;
        start = (start + 1 + rng.below(3) as usize) % clients.len();
        for k in 0..clients.len() {
            let ci = (start + k) % clients.len();
            let c = &clients[ci];
            if sh.now() < next_ready[ci] {
                continue;
            }
            if cfg.stall_p > 0 && rng.p(cfg.stall_p) {
                // a stall: the PBDMA of this channel does nothing for a while (fills the private ring)
                next_ready[ci] = sh.now() + rng.range(1, u64::from(cfg.stall_us_max.max(1))) * 1000;
                continue;
            }
            let burst = rng.range(1, u64::from(cfg.burst.max(1)));
            for _ in 0..burst {
                let item = c.pq.lock().unwrap_or_else(|e| e.into_inner()).pop_front();
                let Some(item) = item else { break };
                did = true;
                chaos();
                if cfg.cost_us > 0 {
                    sleep_or_spin(u64::from(cfg.cost_us));
                }
                match item {
                    Pub::Guest { stored, .. } => {
                        let want = c.exec_abs.load(Ordering::Relaxed);
                        let got = entry_n(stored);
                        if got != want % (1 << 24) && !(want == 0 && got == 0) {
                            sh.violation(format!(
                                "I1 client {ci}: the PBDMA popped guest entry {got} where {want} was due (lost, duplicated or reordered)"
                            ));
                        }
                        c.exec_abs.fetch_add(1, Ordering::AcqRel);
                    }
                    Pub::Barrier => {
                        if cfg.barrier_us_max > 0 {
                            sleep_or_spin(rng.below(u64::from(cfg.barrier_us_max)));
                        }
                    }
                    Pub::Marker(_) | Pub::Staged { .. } => {}
                }
                popped_total[ci] += 1;
                c.hw_get
                    .store((popped_total[ci] as u32) & (PRIV_N - 1), Ordering::Release);
                c.last_pop_ns.store(sh.now(), Ordering::Release);
                if let Pub::Marker(n) = item {
                    chaos();
                    // the marker processed: REFERENCE, then the non-stall interrupt (delayed, reordered across markers)
                    c.hw_ref.store(Marks::value(n), Ordering::Release);
                    c.ref_ns.store(sh.now(), Ordering::Release);
                    let due = sh.now() + rng.below(u64::from(cfg.irq_jitter_us) + 1) * 1000;
                    irqs.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(Due(due));
                    sh.irqs_raised.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        if !did {
            std::thread::sleep(Duration::from_micros(30));
        }
    }
    chaos_remove();
}

fn irq_thread(sh: &Shared, ev: &Notifier, irqs: &Mutex<BinaryHeap<Due>>) {
    while !sh.stop_gpu.load(Ordering::Acquire) {
        let next = {
            let mut q = irqs.lock().unwrap_or_else(|e| e.into_inner());
            match q.peek() {
                Some(Due(d)) if *d <= sh.now() => q.pop().map(|d| d.0),
                Some(Due(d)) => {
                    let wait = d.saturating_sub(sh.now());
                    drop(q);
                    std::thread::sleep(Duration::from_nanos(wait.min(200_000)));
                    continue;
                }
                None => None,
            }
        };
        match next {
            Some(_) => {
                let _ = sh.irq_pending_since.compare_exchange(
                    0,
                    sh.now(),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
                sh.last_raise_ns.store(sh.now(), Ordering::Release);
                let _ = ev.signal();
            }
            None => std::thread::sleep(Duration::from_micros(50)),
        }
    }
}

/// I6: see `run_scenario`. Ground truth from the relay records' own flags (independent of `RelayLoop::unsourced`).
fn sleep_safety_monitor(
    sh: &Shared,
    rt: &Rt<'_>,
    ws: &WorkerStats,
    workers: usize,
    irqs: &Mutex<BinaryHeap<Due>>,
) {
    // an interrupt the fake hardware has raised but not yet delivered is a wake IN FLIGHT, not a missing one
    // a wake IN FLIGHT is not a missing one: an interrupt the fake hardware has queued, or one it raised (eventfd signalled)
    // that no worker has dispatched yet (I6 classification 2026-10-20: the false positives were exactly these)
    let in_flight = || {
        irqs.lock().map_or(true, |q| !q.is_empty())
            || sh.last_raise_ns.load(Ordering::Acquire)
                >= sh.last_dispatch_ns.load(Ordering::Acquire)
                && sh.last_raise_ns.load(Ordering::Acquire) != 0
    };
    let blocked = || {
        ws.parks
            .load(Ordering::Acquire)
            .saturating_sub(sh.after_waits.load(Ordering::Acquire))
            >= workers as u64
    };
    // demand (the retry set), a carried wait, or a recorded wait the hardware words already satisfy (its wake was consumed)
    let obligation = || {
        rt.cells
            .iter()
            .enumerate()
            .find(|(i, c)| {
                c.pending()
                    || (c.is_waiting() && c.carried())
                    || c.try_lock().is_ok_and(|r| {
                        let cl = &rt.clients[*i];
                        r.wait.is_some_and(|w| {
                            r.relay
                                .core
                                .fetch
                                .peek(cl.hw_get.load(Ordering::Acquire))
                                .is_ok_and(|popped| {
                                    w.satisfied(
                                        r.relay.core.marks.peek(cl.hw_ref.load(Ordering::Acquire)),
                                        popped,
                                    )
                                })
                        })
                    })
            })
            .map(|(i, c)| {
                // the obligation's IDENTITY: the same wait (kind and target) must persist across the window
                let wait = c.try_lock().ok().and_then(|r| r.wait);
                (i, c.pending(), c.carried(), wait)
            })
    };
    while !sh.stop_gpu.load(Ordering::Acquire) {
        std::thread::sleep(Duration::from_millis(1));
        if !blocked() {
            continue;
        }
        if in_flight() {
            continue;
        }
        let Some(first) = obligation() else { continue };
        std::thread::sleep(Duration::from_millis(3));
        if blocked()
            && !in_flight()
            && obligation() == Some(first)
            && !sh.stop_gpu.load(Ordering::Acquire)
        {
            let report = |m: String| {
                if env_u("RS_I6_SOFT", 0) != 0 {
                    sh.soft(m)
                } else {
                    sh.violation(m)
                }
            };
            {
                let c = &rt.cells[first.0];
                let cl = &rt.clients[first.0];
                let now = sh.now();
                let forensic = c.try_lock().map(|r| {
                    format!(
                        "wait={:?} armed_ago_us={} last_look={:?} ago_us={:?} need={:?} again={} marks(issued={} seen={}) fetch(issued={} fetched={}) hw_get={} hw_ref_n={:?} ref_store_ago_us={} last_dispatch_ago_us={} last_raise_ago_us={} token={}",
                        r.wait,
                        now.saturating_sub(r.armed_ns) / 1000,
                        r.last_look.map(|l| (l.1, l.2)),
                        r.last_look.map(|l| now.saturating_sub(l.0) / 1000),
                        r.relay.core.need,
                        r.relay.core.again,
                        r.relay.core.marks.issued(),
                        r.relay.core.marks.seen(),
                        r.relay.core.fetch.issued(),
                        r.relay.core.fetch.fetched(),
                        cl.hw_get.load(Ordering::Acquire),
                        r.relay.core.marks.peek(cl.hw_ref.load(Ordering::Acquire)),
                        now.saturating_sub(cl.ref_ns.load(Ordering::Acquire)) / 1000,
                        now.saturating_sub(sh.last_dispatch_ns.load(Ordering::Acquire)) / 1000,
                        now.saturating_sub(sh.last_raise_ns.load(Ordering::Acquire)) / 1000,
                        rt.plane.tokens.get(first.0).map(|w| format!("{:?}", w.load())).unwrap_or_default(),
                    )
                });
                eprintln!("I6 FORENSICS relay {}: {:?}", first.0, forensic);
            }
            report(format!(
                "I6 every worker is in a blocking wait while relay {} holds an obligation with no wake source (demand={} carried={})",
                first.0, first.1, first.2
            ));
            let _ = first.3;
            return;
        }
    }
}

/// The act thread: takes the record / pt / storage locks (blocking) for random holds. Production's act thread rings the token
/// after a record hold (`user_ring_wake`); a pt / storage holder announces nothing.
fn holder_thread(sh: &Shared, rt: &Rt<'_>, cfg: HolderCfg, seed: u64, ch: ChaosCfg) {
    chaos_install(seed, 800, ch);
    let mut rng = Rng::new(seed, 800);
    while !sh.stop_workers.load(Ordering::Acquire) {
        sleep_or_spin(rng.below(u64::from(cfg.gap_us_max) + 1));
        let i = rng.below(rt.cells.len() as u64) as usize;
        match rng.below(4) {
            0 | 1 => {
                let cell = &rt.cells[i];
                {
                    let _g = cell.lock();
                    sleep_or_spin(rng.range(1, u64::from(cfg.hold_us_max.max(1))));
                }
                if rng.p(cfg.rings_after_p) {
                    if cell.request() {
                        cell.wake_if(
                            |_| true,
                            |idx| {
                                rt.ring_token(idx);
                            },
                        );
                    }
                }
            }
            2 => {
                let _g = sh.pt.lock();
                sleep_or_spin(rng.range(1, u64::from(cfg.hold_us_max.max(1))));
            }
            _ => {
                let _g = rt.clients[i].storage.lock();
                sleep_or_spin(rng.range(1, u64::from((cfg.hold_us_max / 10).max(1))));
            }
        }
    }
    chaos_remove();
}

// ------------------------------------------------------------------------------------------------
// the guests

#[derive(Default)]
struct GuestReport {
    role: Option<Role>,
    batches: u64,
    entries: u64,
    resp: Hist,
    total: Hist,
    full_wait: Hist,
    max_serves_during_resp: u64,
    rings: u64,
    full_events: u64,
    /// Batches answered later than the bound while the workers served other tokens.
    starved: u64,
}

struct Pending {
    abs: u64,
    t_submit: u64,
    t_hw: Option<u64>,
    serves_at_hw: u64,
}

fn guest(
    sh: &Shared,
    plane: &Plane<'_>,
    wake: &Notifier,
    c: &Client,
    role: Role,
    sc: &Scenario,
    deadline: Instant,
) -> GuestReport {
    let seed = sc.seed;
    chaos_install(seed, 100 + u64::from(c.idx), sc.chaos);
    let mut rng = Rng::new(seed, 100 + u64::from(c.idx));
    let mut rep = GuestReport {
        role: Some(role),
        ..Default::default()
    };
    let mut put_abs: u64 = 0;
    let mut gg_abs: u64 = 0;
    let mut last_gg: u32 = 0;
    let mut pend: VecDeque<Pending> = VecDeque::new();
    let hard = Duration::from_millis(env_u("RS_HARD_MS", 5_000));
    let ring = |rep: &mut GuestReport, style_dup: bool| {
        let go = || {
            if plane.trap_write(Class::Doorbell, 0, DB_OFF, u64::from(c.idx), 4)
                == Action::WakeWorker
            {
                let _ = wake.signal();
            }
        };
        go();
        rep.rings += 1;
        if style_dup {
            go();
            if rng_dup(c.idx) {
                go();
            }
        }
    };
    fn rng_dup(i: u32) -> bool {
        i % 2 == 0
    }
    // Poll completions: updates gg_abs, pending bookkeeping, latency, I2.
    let poll = |gg_abs: &mut u64,
                last_gg: &mut u32,
                pend: &mut VecDeque<Pending>,
                rep: &mut GuestReport| {
        let gg = c.gp_get.load(Ordering::Acquire);
        *gg_abs += u64::from(gg.wrapping_sub(*last_gg) & (GUEST_N - 1));
        *last_gg = gg;
        let exec = c.exec_abs.load(Ordering::Acquire);
        let now = sh.now();
        for p in pend.iter_mut() {
            if p.t_hw.is_none() && exec >= p.abs {
                p.t_hw = Some(now);
                p.serves_at_hw = sh.serves.load(Ordering::Relaxed);
            }
        }
        while let Some(p) = pend.front() {
            if *gg_abs < p.abs {
                break;
            }
            let t_hw = p.t_hw.unwrap_or(now);
            let resp = now.saturating_sub(t_hw);
            let total = now.saturating_sub(p.t_submit);
            rep.resp.add(resp);
            rep.total.add(total);
            let served_meanwhile = sh
                .serves
                .load(Ordering::Relaxed)
                .saturating_sub(p.serves_at_hw);
            rep.max_serves_during_resp = rep.max_serves_during_resp.max(served_meanwhile);
            // STARVED = slow while the workers kept serving OTHER tokens (a loaded dev box makes everything slow, but the workers then
            // serve little; the baton is slow while they serve millions)
            if resp > sh.bound_ns && served_meanwhile > sh.starve_serves {
                rep.starved += 1;
            }
            if resp > sh.bound_ns && served_meanwhile > sh.starve_serves && rep.starved == 1 {
                sh.soft(format!(
                    "I2 client {} ({:?}): the relay answered {:.1} ms after the hardware finished a batch (bound {:.0} ms); the workers served {} tokens meanwhile",
                    c.idx,
                    rep.role,
                    resp as f64 / 1e6,
                    sh.bound_ns as f64 / 1e6,
                    served_meanwhile
                ));
            }
            rep.batches += 1;
            pend.pop_front();
        }
    };
    let wait_for = |abs: u64,
                    gg_abs: &mut u64,
                    last_gg: &mut u32,
                    pend: &mut VecDeque<Pending>,
                    rep: &mut GuestReport,
                    what: &str|
     -> bool {
        // STUCK = no progress of the guest-visible GET for `hard` (a slow run on a loaded box is not stuck)
        let mut t = Instant::now();
        let mut seen_gg = *gg_abs;
        loop {
            poll(gg_abs, last_gg, pend, rep);
            if *gg_abs >= abs {
                return true;
            }
            if sh.abort.load(Ordering::Acquire) {
                return false;
            }
            if *gg_abs != seen_gg {
                seen_gg = *gg_abs;
                t = Instant::now();
            }
            if t.elapsed() > hard {
                sh.violation(format!(
                    "I2/I3 client {} ({:?}): {what}: GP_GET stuck at {} (< {abs}) for {:?}; exec={} published={} (the guest would block for ever)",
                    c.idx,
                    rep.role,
                    *gg_abs,
                    t.elapsed(),
                    c.exec_abs.load(Ordering::Relaxed),
                    c.published.load(Ordering::Relaxed)
                ));
                return false;
            }
            chaos();
            sleep_or_spin(40);
        }
    };

    let mut next_n: u64 = 0;
    let mut iter = 0u64;
    'outer: while Instant::now() < deadline && !sh.abort.load(Ordering::Acquire) {
        iter += 1;
        let kind = match role {
            Role::Mixed => [Role::Hot, Role::Cold, Role::Filler][rng.below(3) as usize],
            r => r,
        };
        let (count, wait_done, think_us) = match kind {
            Role::Hot => (rng.range(8, 256), false, 0),
            Role::Cold => (rng.range(1, 3), true, rng.range(100, 5000)),
            Role::Filler => (rng.range(2000, u64::from(GUEST_N) - 1), false, 0),
            Role::Mixed => unreachable!(),
        };
        let mut written = 0;
        let mut rung_since_put = true;
        while written < count {
            // the ring is FULL when PUT+1 == GET (the guest-visible one)
            let put = c.gp_put.load(Ordering::Relaxed);
            poll(&mut gg_abs, &mut last_gg, &mut pend, &mut rep);
            if put.wrapping_add(1) & (GUEST_N - 1) == c.gp_get.load(Ordering::Acquire) {
                // I3: the guest blocks on GET after ringing for what it already published
                if !rung_since_put {
                    ring(&mut rep, false);
                    rung_since_put = true;
                }
                rep.full_events += 1;
                let t = Instant::now();
                let target = gg_abs + 1;
                if !wait_for(
                    target,
                    &mut gg_abs,
                    &mut last_gg,
                    &mut pend,
                    &mut rep,
                    "ring full",
                ) {
                    break 'outer;
                }
                rep.full_wait.add(t.elapsed().as_nanos() as u64);
                continue;
            }
            // a run of entries up to the next full slot, one PUT store
            let room = (c
                .gp_get
                .load(Ordering::Acquire)
                .wrapping_sub(put)
                .wrapping_sub(1)
                & (GUEST_N - 1)) as u64;
            let run = (count - written).min(room.max(1)).min(rng.range(1, 512));
            for k in 0..run {
                let slot = put.wrapping_add(k as u32) & (GUEST_N - 1);
                let sync = sc.sync_p > 0 && next_n > 0 && rng.p(sc.sync_p);
                let raw = entry_raw(next_n, sync);
                c.ring_lo[slot as usize].store(raw as u32, Ordering::Release);
                c.ring_hi[slot as usize].store((raw >> 32) as u32, Ordering::Release);
                next_n += 1;
            }
            written += run;
            put_abs += run;
            chaos(); // between the entry stores and the PUT store
            c.gp_put.store(
                put.wrapping_add(run as u32) & (GUEST_N - 1),
                Ordering::Release,
            );
            rung_since_put = false;
            pend.push_back(Pending {
                abs: put_abs,
                t_submit: sh.now(),
                t_hw: None,
                serves_at_hw: 0,
            });
            rep.entries += run;
            chaos(); // between the PUT store and the doorbell
            let last_run = written >= count;
            let style = rng.below(1024) as u32;
            if style < sc.style_late_p {
                sleep_or_spin(rng.range(1, 300));
            }
            let skip = !last_run && !wait_done && style > 1024 - sc.style_skip_p;
            if !skip {
                ring(&mut rep, style % 1024 < sc.style_dup_p);
                rung_since_put = true;
            }
        }
        if !rung_since_put {
            ring(&mut rep, false);
        }
        if wait_done {
            if !wait_for(
                put_abs,
                &mut gg_abs,
                &mut last_gg,
                &mut pend,
                &mut rep,
                "cold wait",
            ) {
                break 'outer;
            }
        } else if kind == Role::Filler {
            // wait until half a ring has drained
            let target = put_abs.saturating_sub(u64::from(GUEST_N) / 2);
            if !wait_for(
                target,
                &mut gg_abs,
                &mut last_gg,
                &mut pend,
                &mut rep,
                "filler drain",
            ) {
                break 'outer;
            }
        }
        if think_us > 0 {
            sleep_or_spin(think_us);
        }
        let _ = iter;
    }
    // final drain: everything published must be completed (guest GET covers PUT)
    if !sh.abort.load(Ordering::Acquire) {
        ring(&mut rep, false);
        let _ = wait_for(
            put_abs,
            &mut gg_abs,
            &mut last_gg,
            &mut pend,
            &mut rep,
            "final drain",
        );
    }
    chaos_remove();
    rep
}

// ------------------------------------------------------------------------------------------------
// the run

#[derive(Debug)]
struct Outcome {
    seed: u64,
    violations: Vec<String>,
    soft: Vec<String>,
    summary: String,
    entries: u64,
    serves: u64,
    resp_max_ms: f64,
    resp_p99_us: u64,
    stats_line: String,
    backstop_found_work: u64,
    backstop_contended: u64,
    timeouts_with_work: u64,
    busy_looks: u64,
    per_client: Vec<String>,
}

fn bound_ms() -> u64 {
    std::env::var("RS_BOUND_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(250)
}

/// RS_DUMP=1: the state of every relay at the moment a run was aborted (before the workers are stopped).
fn dump_state(rt: &Rt<'_>, clients: &[Arc<Client>]) {
    for (i, cell) in rt.cells.iter().enumerate() {
        let c = &clients[i];
        if let Ok(r) = cell.try_lock() {
            let k = &r.relay.core;
            eprintln!(
                "DUMP relay {i}: need={:?} wait={:?} again={} dead={:?} cursor={} put={} gp_get={} written_get={} guest_get_owed={:?} fetch(issued={} fetched={} avail={}) marks(issued={} seen={}) hw_get={} hw_ref={:#x} pq={} unnotified={} marker_event={} gate_target={} sync_waiting={} cell(retry={} waiting={} contended_run={} canceled={})",
                k.need,
                r.wait,
                k.again,
                r.dead,
                k.cursor,
                c.gp_put.load(Ordering::Relaxed),
                c.gp_get.load(Ordering::Relaxed),
                k.written_get,
                k.guest_get,
                k.fetch.issued(),
                k.fetch.fetched(),
                k.fetch.available(),
                k.marks.issued(),
                k.marks.seen(),
                c.hw_get.load(Ordering::Relaxed),
                c.hw_ref.load(Ordering::Relaxed),
                c.pq.lock().map(|q| q.len()).unwrap_or(0),
                k.unnotified,
                k.marker_event,
                k.gate_target,
                k.sync.waiting(),
                cell.retry.load(Ordering::Relaxed),
                cell.waiting.load(Ordering::Relaxed),
                cell.contended_run(),
                cell.is_canceled(),
            );
        }
    }
}

fn run_scenario(sc: &Scenario) -> Outcome {
    #[cfg(feature = "chaos-hooks")]
    kf_trap::chaos::set_hook(hook);
    // ★ 2026-10-20 (owner): designed and tested as if the park were INFINITE; `RS_PARK_NET=1` restores production's 50 ms net.
    worker::set_park_net(env_u("RS_PARK_NET", 0) != 0);
    let n = sc.roles.len();
    let vmm = Vmm::default();
    let plane = Plane::new(&vmm, 16, 15);
    let mut caps = VmCaps::from_declared(16, 8, 8, 8);
    for i in 0..(n + sc.synth_hot) as u32 {
        plane
            .allocate_channel(&mut caps, i, Route::Translated, i, Owner::User)
            .expect("allocate");
    }
    let sh = Shared {
        t0: Instant::now(),
        stop_gpu: AtomicBool::new(false),
        stop_workers: AtomicBool::new(false),
        after_waits: Arc::new(AtomicU64::new(0)),
        last_dispatch_ns: AtomicU64::new(0),
        last_raise_ns: AtomicU64::new(0),
        abort: AtomicBool::new(false),
        violations: Mutex::new(Vec::new()),
        soft: Mutex::new(Vec::new()),
        serves: AtomicU64::new(0),
        forges: AtomicU64::new(0),
        irq_pending_since: AtomicU64::new(0),
        irq_hist: Mutex::new(Hist::default()),
        irqs_raised: AtomicU64::new(0),
        irqs_dispatched: AtomicU64::new(0),
        bound_ns: bound_ms() * 1_000_000,
        starve_serves: env_u("RS_STARVE_SERVES", 20_000),
        pt: Mutex::new(()),
    };
    let wake = Notifier::create().expect("eventfd");
    let ev = Notifier::create().expect("engine event fd");
    let comp_fd = Notifier::create().expect("completion fd");
    let stats = RelayStats::default();
    let ws = WorkerStats::default();
    ws.prof.store(true, Ordering::Relaxed);
    let clients: Vec<Arc<Client>> = (0..n as u32).map(|i| Arc::new(Client::new(i))).collect();
    let mut cells = Vec::new();
    for i in 0..n as u32 {
        let life = if sc.always_scan {
            Lifecycle::always_scan()
        } else {
            Lifecycle::new()
        };
        let relay = Relay::new(
            PRIV_N,
            life,
            Scan::for_subdevice(1).unwrap(),
            Glue::default(),
        )
        .with_lazy(kf_chan::user_lazy::LazyConfig::ON);
        cells.push(Arc::new(Cell::new(
            i,
            Rec {
                relay,
                env_rng_seed: sc.seed ^ (u64::from(i) << 40),
                wait: None,
                dead: None,
                turns: 0,
                armed_ns: 0,
                last_look: None,
            },
        )));
    }
    let rt = Rt {
        plane: &plane,
        sh: &sh,
        clients: clients.clone(),
        rings: Mutex::new(cells.clone()),
        cells,
        wake: &wake,
        stats: &stats,
        busy_p: sc.busy_p,
        turn_us: env_u("RS_TURN_US", sc.turn_us),
        n_real: n as u32,
        serve_calls: AtomicU64::new(0),
    };
    let irqs: Mutex<BinaryHeap<Due>> = Mutex::new(BinaryHeap::new());
    for k in 0..sc.synth_hot as u32 {
        rt.ring_token(n as u32 + k);
    }
    let t_start = Instant::now();
    let deadline = t_start + Duration::from_secs_f64(sc.secs);
    let reports: Mutex<Vec<(usize, GuestReport)>> = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        // workers
        let mut worker_handles = Vec::new();
        for wi in 0..sc.workers {
            let (rt, sh, plane, wake, ev, comp_fd, ws, stats) =
                (&rt, &sh, &plane, &wake, &ev, &comp_fd, &ws, &stats);
            let sc = sc.clone();
            worker_handles.push(s.spawn(move || {
                chaos_install(sc.seed, 10 + wi as u64, sc.chaos);
                AFTER_WAITS.with(|a| *a.borrow_mut() = Some(sh.after_waits.clone()));
                let poller = Poller::create().expect("epoll");
                poller
                    .watch(wake.as_source_fd(), worker::WORKER_EFD_TAG)
                    .unwrap();
                poller
                    .watch(comp_fd.as_source_fd(), worker::COMPLETIONS_TAG)
                    .unwrap();
                poller
                    .watch(ev.as_source_fd(), worker::OTHER_TAG_BASE)
                    .unwrap();
                let on_other = |tag: u64| {
                    if tag == worker::TICK_TAG {
                        return;
                    }
                    // the engine's non-stall event: the guest's completion interrupt would be raised here
                    let _ = ev.drain();
                    sh.last_dispatch_ns.store(sh.now(), Ordering::Release);
                    let since = sh.irq_pending_since.swap(0, Ordering::AcqRel);
                    if since != 0 {
                        let lat = sh.now().saturating_sub(since);
                        sh.irq_hist
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .add(lat);
                        sh.irqs_dispatched.fetch_add(1, Ordering::Relaxed);
                    }
                };
                worker::run_with_relay(
                    plane,
                    rt,
                    &poller,
                    wake,
                    &NoInflight,
                    ws,
                    &sh.stop_workers,
                    &on_other,
                    &|| {},
                    &|| false,
                    rt,
                    stats,
                );
                chaos_remove();
            }));
        }
        let gpu = s.spawn(|| gpu_thread(&sh, &clients, sc.gpu, &irqs, sc.seed, sc.chaos));
        let irq = s.spawn(|| irq_thread(&sh, &ev, &irqs));
        // ★ I6 SLEEP SAFETY (2026-10-20): every worker in a blocking wait while a relay holds an obligation no registered wake
        // source announces (demand left behind = the retry set, or a `Wait::Popped`), seen twice 3 ms apart = a violation.
        let monitor = s.spawn(|| sleep_safety_monitor(&sh, &rt, &ws, sc.workers, &irqs));
        let holder = sc
            .holder
            .on
            .then(|| s.spawn(|| holder_thread(&sh, &rt, sc.holder, sc.seed, sc.chaos)));
        let mut guest_handles = Vec::new();
        for (i, role) in sc.roles.iter().enumerate() {
            let (sh, plane, wake, c) = (&sh, &plane, &wake, &clients[i]);
            let role = *role;
            let reports = &reports;
            guest_handles.push(s.spawn(move || {
                let r = guest(sh, plane, wake, c, role, sc, deadline);
                reports.lock().unwrap().push((i, r));
            }));
        }
        for g in guest_handles {
            if g.join().is_err() {
                sh.violation("I5 a guest thread panicked".into());
            }
        }
        if sh.abort.load(Ordering::Acquire) && std::env::var("RS_DUMP").is_ok() {
            dump_state(&rt, &clients);
        }
        // the guests are done; let the hardware finish (I1: everything published was executed), then stop
        if !sh.abort.load(Ordering::Acquire) {
            let t = Instant::now();
            while t.elapsed() < Duration::from_secs(5) {
                if clients
                    .iter()
                    .all(|c| c.pq.lock().map(|q| q.is_empty()).unwrap_or(true))
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        // ★ I7 NO HIDDEN SPIN: with everything executed and nothing outstanding, the workers block and stay blocked (infinite
        // park): no pass, no park, no timeout-0 look in a quiet window.
        // Precondition of "nothing outstanding": no synthetic hot token, no lock-holder thread (it keeps acting), and every guest's
        // visible GP_GET at its GP_PUT (else the relay still owes reads or write-backs).
        let settled = || {
            clients
                .iter()
                .all(|c| c.gp_get.load(Ordering::Acquire) == c.gp_put.load(Ordering::Acquire))
                // no recorded wait (a batch marker's late interrupt legitimately rings one more turn) and no interrupt in flight
                && rt.cells.iter().all(|c| !c.is_waiting())
                && irqs.lock().map_or(false, |q| q.is_empty())
        };
        if !sh.abort.load(Ordering::Acquire)
            && env_u("RS_PARK_NET", 0) == 0
            && sc.synth_hot == 0
            && !sc.holder.on
            && settled()
        {
            std::thread::sleep(Duration::from_millis(20));
            let quiet = |ws: &WorkerStats, st: &RelayStats| {
                (
                    ws.parks.load(Ordering::Relaxed),
                    st.retry_looks.load(Ordering::Relaxed),
                    rt.serve_calls.load(Ordering::Relaxed),
                )
            };
            let before = quiet(&ws, &stats);
            std::thread::sleep(Duration::from_millis(30));
            let after = quiet(&ws, &stats);
            if before != after
                && clients
                    .iter()
                    .all(|c| c.pq.lock().map(|q| q.is_empty()).unwrap_or(true))
            {
                sh.violation(format!(
                    "I7 the workers were not quiet with nothing outstanding: (parks, timeout-0 looks, serves) {before:?} -> {after:?}"
                ));
            }
        }
        sh.stop_gpu.store(true, Ordering::Release);
        sh.stop_workers.store(true, Ordering::Release);
        let _ = wake.signal();
        let _ = ev.signal();
        // keep waking until the workers are gone (a worker may be mid-wait)
        let t = Instant::now();
        while worker_handles.iter().any(|h| !h.is_finished())
            && t.elapsed() < Duration::from_secs(5)
        {
            let _ = wake.signal();
            std::thread::sleep(Duration::from_millis(1));
        }
        for w in worker_handles {
            if w.join().is_err() {
                sh.violation("I5 a worker thread panicked".into());
            }
        }
        if gpu.join().is_err() {
            sh.violation("I5 the fake GPU thread panicked".into());
        }
        if irq.join().is_err() {
            sh.violation("I5 the fake interrupt thread panicked".into());
        }
        if monitor.join().is_err() {
            sh.violation("I5 the sleep-safety monitor panicked".into());
        }
        if let Some(h) = holder {
            let _ = h.join();
        }
    });
    let elapsed = t_start.elapsed();
    // ---- end-of-run invariants
    let mut reps = reports.into_inner().unwrap();
    reps.sort_by_key(|r| r.0);
    for (i, c) in clients.iter().enumerate() {
        let put = c.gp_put.load(Ordering::Acquire);
        let rep_entries = reps.iter().find(|r| r.0 == i).map_or(0, |r| r.1.entries);
        if !sh.abort.load(Ordering::Acquire) {
            let exec = c.exec_abs.load(Ordering::Acquire);
            if exec != rep_entries {
                sh.violation(format!("I1 client {i}: the guest wrote {rep_entries} entries, the PBDMA executed {exec}"));
            }
            if c.gp_get.load(Ordering::Acquire) != put {
                sh.violation(format!(
                    "I2 client {i}: the guest's GP_GET {} did not reach GP_PUT {put} at the end",
                    c.gp_get.load(Ordering::Acquire)
                ));
            }
        }
    }
    let g = |a: &AtomicU64| a.load(Ordering::Relaxed);
    let served = g(&ws.served);
    let calls = rt.serve_calls.load(Ordering::Relaxed);
    if served != calls {
        sh.violation(format!(
            "I4 WorkerStats.served {served} != run_translated calls {calls}"
        ));
    }
    if g(&stats.busy_looks) > served {
        sh.violation(format!(
            "I4 busy_looks {} > served {served} (a busy look is one per busy pass)",
            g(&stats.busy_looks)
        ));
    }
    if g(&stats.backstop_found_work) > 0 {
        sh.violation(format!(
            "I4 backstop_found_work = {}: the safety net found a satisfied marker wait an interrupt should have announced (a lost wake)",
            g(&stats.backstop_found_work)
        ));
    }
    if g(&ws.timeouts_with_work) > 0 {
        sh.soft(format!(
            "I4 timeouts_with_work = {}: a park timeout (the 50 ms safety net) found work -- a lost or late wake",
            g(&ws.timeouts_with_work)
        ));
    }
    if g(&stats.backstop_contended) > 0 {
        sh.soft(format!(
            "I4 backstop_contended = {}: a contended relay was re-rung ONLY by the 50 ms safety net (the retry budget was exhausted)",
            g(&stats.backstop_contended)
        ));
    }
    let mut resp = Hist::default();
    let mut per_client = Vec::new();
    let mut entries = 0;
    for (i, r) in &reps {
        resp.merge(&r.resp);
        entries += r.entries;
        per_client.push(format!(
            "client {i} {:?}: starved_batches={} batches={} entries={} rings={} full_events={} | resp {} | total {} | full_wait {} | max serves during a resp {}",
            r.role.unwrap(),
            r.starved,
            r.batches,
            r.entries,
            r.rings,
            r.full_events,
            r.resp.line(),
            r.total.line(),
            r.full_wait.line(),
            r.max_serves_during_resp
        ));
    }
    let irq_line = sh.irq_hist.lock().unwrap().line();
    let stats_line = format!(
        "{} | worker(served={} parks={} timeouts={} timeouts_with_work={} host_rings={}) | irq raised={} dispatched={} lat {}",
        stats.summary().split(" drain_us_log2").next().unwrap_or(""),
        served,
        g(&ws.parks),
        g(&ws.timeouts),
        g(&ws.timeouts_with_work),
        g(&ws.host_rings),
        sh.irqs_raised.load(Ordering::Relaxed),
        sh.irqs_dispatched.load(Ordering::Relaxed),
        irq_line
    );
    let violations = sh.violations.lock().unwrap().clone();
    let soft = sh.soft.lock().unwrap().clone();
    Outcome {
        seed: sc.seed,
        violations,
        soft,
        summary: format!(
            "seed {} clients={:?} workers={} scan={} sync_p={} gpu={:?} holder={:?} busy_p={} chaos={:?} wall={:.2}s",
            sc.seed,
            sc.roles,
            sc.workers,
            sc.always_scan,
            sc.sync_p,
            sc.gpu,
            sc.holder,
            sc.busy_p,
            sc.chaos,
            elapsed.as_secs_f64()
        ),
        entries,
        serves: served,
        resp_max_ms: resp.max_ns as f64 / 1e6,
        resp_p99_us: resp.p(0.99),
        stats_line,
        backstop_found_work: g(&stats.backstop_found_work),
        backstop_contended: g(&stats.backstop_contended),
        timeouts_with_work: g(&ws.timeouts_with_work),
        busy_looks: g(&stats.busy_looks),
        per_client,
    }
}

fn report(o: &Outcome) {
    println!("{}", o.summary);
    for l in &o.per_client {
        println!("    {l}");
    }
    println!("    loop: {}", o.stats_line);
    println!(
        "    entries={} serves={} resp max {:.2} ms p99<={}us",
        o.entries, o.serves, o.resp_max_ms, o.resp_p99_us
    );
    for v in o.violations.iter().chain(o.soft.iter()) {
        println!("    VIOLATION {v}");
    }
}

/// `RS_STRICT=1`: the SOFT class (starvation behind a contended `try_lock`, safety-net findings) fails a run too.
fn failed_run(o: &Outcome) -> bool {
    !o.violations.is_empty() || (env_u("RS_STRICT", 0) != 0 && !o.soft.is_empty())
}

fn env_f(name: &str, d: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(d)
}
fn env_u(name: &str, d: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(d)
}

// ------------------------------------------------------------------------------------------------
// tests

/// The default-suite smoke: a handful of seeds, ~0.6 s each, every invariant on.
#[test]
fn smoke_a_few_seeds_hold_every_invariant() {
    let secs = env_f("RS_SMOKE_SECS", 0.6);
    for seed in [2u64, 3, 5] {
        let o = run_scenario(&Scenario::from_seed(seed, secs));
        if !o.violations.is_empty() || !o.soft.is_empty() {
            report(&o);
        }
        assert!(
            !failed_run(&o),
            "seed {seed}: {:?} {:?}",
            o.violations,
            o.soft
        );
        assert!(o.entries > 0, "seed {seed}: nothing ran");
    }
}

/// ★ The baton, in the default suite: two workers, four synthetic hot tokens (a relay turn costs 100 us, as the real pump's guarded
/// accessors do) and one cold sync client. WITHOUT the fairness fix (`AfterPass::Look` replaced by `continue`, scripts/relay_stress_mutants.sh
/// `control_look_removed`) the cold client's answer takes 2.6-2.8 s (MEASURED, 2 of 2 seeds); WITH it ~10 ms.
#[test]
fn the_baton_a_cold_client_is_answered_beside_hot_neighbours() {
    for seed in [1u64, 2] {
        let o = run_scenario(&Scenario::baton(seed, 2.0));
        if !o.violations.is_empty() || o.soft.iter().any(|v| v.contains("I2")) {
            report(&o);
        }
        assert!(o.violations.is_empty(), "seed {seed}: {:?}", o.violations);
        assert!(
            !o.soft.iter().any(|v| v.contains("I2")),
            "seed {seed}: a cold client starved beside hot neighbours: {:?}",
            o.soft
        );
        assert!(o.entries > 20, "seed {seed}: the cold client barely ran");
    }
}

/// ★ EXPECTED RED (2026-10-20, the case a model agent is fixing): with the fairness fix in place, two workers still contend on a relay's
/// record lock without any act thread: one worker's LOOK (`peek_one`) holds the record `try_lock` while the other worker's SERVE
/// (`user_ring_serve`, `record.enter()`) tries it, loses, and counts a contended turn; after `CONTENTION_RETRIES` (64) such losses the spin
/// and event looks no longer re-ring the relay (`may_retry`), only the 50 ms backstop does, and a busy worker never reaches the backstop.
/// MEASURED (6 of 6 seeds, 2 real hot clients + 1 cold, no holder thread): `backstop_contended` 10-30 and `timeouts_with_work` 6-69 per
/// 2.5 s run, a HOT client answered up to 2.4 s late while the workers served up to 600 000 tokens meanwhile. CODE: `userpump.rs`
/// `user_ring_serve` (the `record.enter()` failure arm and the `pt.try_lock()` arm both `contended_note` and return), `user_ring_peek_one`
/// (`retry_ok = may_retry(record.contended_run(), why)`), `user_loop.rs` `may_retry` / `CONTENTION_RETRIES`.
/// ★ 2026-10-20 (simple relay): GREEN and un-ignored. The retry budget is deleted; the park is INFINITE in this harness, so a
/// relay re-rung only by a timer would hang the run (HARD). The old assertion (`backstop_contended == 0`) is kept and two that
/// do not depend on the deleted counter are added: no SOFT finding (I2 bounded response) and no timeout found work.
#[test]
fn expected_red_exhausted_try_lock_retries_leave_a_relay_to_the_backstop() {
    let mut worst = 0f64;
    let mut contended = 0;
    for seed in 0..4u64 {
        let mut sc = Scenario::baton(seed, 2.5);
        sc.roles = vec![Role::Hot, Role::Hot, Role::Cold];
        sc.synth_hot = 0;
        sc.turn_us = 0;
        let o = run_scenario(&sc);
        worst = worst.max(o.resp_max_ms);
        contended += o.backstop_contended;
        assert!(o.violations.is_empty(), "seed {seed}: {:?}", o.violations);
        assert!(o.soft.is_empty(), "seed {seed}: {:?}", o.soft);
        assert_eq!(o.timeouts_with_work, 0, "seed {seed}");
    }
    println!("worst response {worst:.1} ms, backstop_contended total {contended}");
    assert_eq!(
        contended, 0,
        "a contended relay was re-rung only by the 50 ms safety net"
    );
}

/// ★ EXPECTED RED, UNCLASSIFIED (2026-10-20): `backstop_found_work > 0` -- the 50 ms safety net found a satisfied marker wait that no
/// look had rung (a lost or never-delivered announcement). MEASURED: seed 18 (2 workers, 7 clients, SYNC_WAIT on 1/4 of the entries, scan
/// mode, fast GPU with short stalls) shows 1-2 per 0.25 s run in 2 of 4 runs. HYPOTHESIS (not proven): the same family as the exhausted
/// try_lock above -- a look that finds the record lock held by the serving worker rings only while `may_retry`, and `contended_run` is
/// far past 64 in this run (`contended_turns` 335-747). FALSIFIER: the counter stays 0 in 20 runs of this seed after the exhausted-retry
/// repair; if it does not, the loss is somewhere else (the arm/peek ordering in `user_ring_arm`).
/// ★ 2026-10-20 (simple relay): GREEN and un-ignored (infinite park: a wait only a timer finds would hang the run; with
/// `RS_PARK_NET=1` the counter below is the check).
#[test]
fn expected_red_seed_18_marker_wait_found_only_by_the_backstop() {
    let mut found = 0;
    for _ in 0..6 {
        let o = run_scenario(&Scenario::from_seed(18, 0.25));
        assert!(o.violations.is_empty(), "{:?}", o.violations);
        found += o.backstop_found_work;
    }
    assert_eq!(found, 0, "backstop_found_work over 6 runs of seed 18");
}

/// ★ 2026-10-20: the seeds on which I6 fired before its classification (0, 1, 3, 18: monitor false positives -- a wake in
/// flight and an obligation compared without its identity; 20: the real busy-record lost wake fixed in `aad1e9ff4`). They
/// must hold every invariant, I6 included, with the infinite park.
#[test]
fn i6_classified_seeds_hold_the_sleep_invariant() {
    for seed in [0u64, 1, 3, 18, 20] {
        let o = run_scenario(&Scenario::from_seed(seed, 0.3));
        if !o.violations.is_empty() {
            report(&o);
        }
        assert!(o.violations.is_empty(), "seed {seed}: {:?}", o.violations);
    }
}

/// Many seeds (the long run). `RS_FROM`, `RS_TO`, `RS_SECS` (per seed), `RS_PRESET=baton|contention|random`.
#[test]
#[ignore = "long: RS_FROM/RS_TO seeds"]
fn many_seeds() {
    let (from, to) = (env_u("RS_FROM", 0), env_u("RS_TO", 100));
    let secs = env_f("RS_SECS", 0.5);
    let preset = std::env::var("RS_PRESET").unwrap_or_else(|_| "random".into());
    let verbose = env_u("RS_VERBOSE", 0) != 0;
    let t0 = Instant::now();
    let mut failed = Vec::new();
    let mut soft_seeds = Vec::new();
    let mut resp_max = 0f64;
    let mut total_entries = 0;
    for seed in from..to {
        let sc = match preset.as_str() {
            "baton" => Scenario::baton(seed, secs),
            "contention" => Scenario::contention(seed, secs),
            _ => Scenario::from_seed(seed, secs),
        };
        let o = run_scenario(&sc);
        resp_max = resp_max.max(o.resp_max_ms);
        total_entries += o.entries;
        if !o.soft.is_empty() {
            soft_seeds.push(seed);
        }
        if failed_run(&o) {
            println!("FAIL seed {seed}");
            report(&o);
            failed.push(seed);
        } else if verbose || !o.soft.is_empty() && env_u("RS_SHOW_SOFT", 0) != 0 {
            report(&o);
        }
    }
    let n = (to - from) as f64;
    println!(
        "RELAY_STRESS preset={preset} seeds={}..{} secs/seed={secs} failed={} {:?} soft_only={} {:?} resp_max={resp_max:.2}ms entries={total_entries} wall={:.1}s => {:.0} seeds/hour",
        from,
        to,
        failed.len(),
        failed,
        soft_seeds.len(),
        soft_seeds,
        t0.elapsed().as_secs_f64(),
        n / t0.elapsed().as_secs_f64() * 3600.0
    );
    assert!(failed.is_empty(), "failed seeds {failed:?}");
}

/// One seed, verbose (`RS_SEED`, `RS_SECS`, `RS_PRESET`).
#[test]
#[ignore = "one seed, verbose"]
fn one_seed() {
    let seed = env_u("RS_SEED", 1);
    let secs = env_f("RS_SECS", 3.0);
    let sc = match std::env::var("RS_PRESET").as_deref() {
        Ok("baton") => Scenario::baton(seed, secs),
        Ok("contention") => Scenario::contention(seed, secs),
        _ => Scenario::from_seed(seed, secs),
    };
    let o = run_scenario(&sc);
    report(&o);
    assert!(!failed_run(&o));
}

/// The glue mirrored in this file is pinned to the production lines it transliterates: when one of them changes the mirror must be
/// re-read (the test names the line).
#[test]
fn a_source_pin_the_mirrored_glue_still_matches_production() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../kf-qemu/src/userpump.rs"
    ))
    .unwrap();
    let relay = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../kf-qemu/src/userrelay.rs"
    ))
    .unwrap();
    for pin in [
        "let Ok(mut r) = record.enter() else {",
        "Err(std::sync::TryLockError::WouldBlock) => {\n                record.contended_note();\n                return Some(false);",
        "let woke = record.retry.swap(false, Ordering::AcqRel);",
        "Err(Failure::Retry) => self.core.need = Need::Contended,",
        "Ok(_) if self.core.again => self.core.need = Need::Again,",
        "Need::Again => {\n                record.contended_clear();\n                if self.plane.ring_internal(idx) {\n                    let _ = self.wake.signal();\n                }\n            }",
        "record.contended_note();\n                let _ = record.request();",
        "record.set_carried(p.wait.is_some_and(kf_chan::user_loop::Wait::carried));",
        "return matches!(look(i), Look::Ring { .. }) && ring(self);",
        "get_follows_pops: kf_chan::user_turn::guest_get_follows_pops(),",
        "record.set_waiting(false);\n                    record.set_carried(false);",
        "record.pending() || (record.is_waiting() && record.carried())",
        "let storage = self\n            .r\n            .lease\n            .storage\n            .lock()",
        "Err(kf_rm::memory_list::AccessError::Busy) => Ok(false),",
    ] {
        assert!(
            src.contains(pin),
            "userpump.rs no longer contains the mirrored line: {pin}"
        );
    }
    for pin in [
        "self.inner.try_lock().inspect_err(|e| {",
        "pub fn contended_note(&self) {",
        "if self.canceled.load(Ordering::Acquire) {\n            return false;\n        }\n        self.retry.store(true, Ordering::Release);",
    ] {
        assert!(
            relay.contains(pin),
            "userrelay.rs no longer contains the mirrored line: {pin}"
        );
    }
}
