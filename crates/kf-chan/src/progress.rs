//! ★ 2026-10-20 -- **REQUIRED VISIT POINTS** (`KF3_DIAG_PROGRESS=<ms>`, default off, measurement only).
//!
//! `docs/design/V3_USER_RING_EVENT_LOOP_20261014.md` section 11. The existing stall detector is a CONSEQUENCE detector (GET != PUT for a
//! while) and stayed silent in every measured starvation. This module instruments the cause: the worker loop and the relay path name
//! the places a BUSY system must keep visiting (a named const table, [`TABLE`]); each place stamps a relaxed "last hit" and a hit
//! counter ([`Board`]). A diagnostic thread (never a vCPU, never the drainer) reads the board and reports
//!  * `PROGRESS_STARVED point=<name> age_ms=<hex>`: a point that a busy worker must visit has not been visited for the threshold
//!    while the worker kept serving ([`Monitor`]);
//!  * a per-worker bound (`passes since the last poll`) and a stuck-in-a-pass check ([`Monitor`]);
//!  * the SHARP one ([`Tracker`]): a parked relay whose wait is SATISFIED (`Wait::satisfied(marks, popped)`) but that no look has
//!    evaluated for the threshold, one bounded block per relay per condition ([`render_relay_block`]), re-armed by a peek.
//!
//! Everything here is pure or atomics: no clock read except [`now_ns`], no lock, no syscall, no allocation on the worker hot path
//! (the worker only calls [`Tap`], which does relaxed stores). Output is rendered by pure functions and is hex-only and bounded.

use crate::user_loop::{Peek, Wait};
use std::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering};

/// The environment variable (milliseconds; unset, empty or `0` = off).
pub const ENV: &str = "KF3_DIAG_PROGRESS";
/// The largest accepted threshold (one hour); anything above is [`Setting::Invalid`].
pub const MAX_THRESHOLD_MS: u64 = 3_600_000;
/// Workers the board has cells for.
pub const MAX_WORKERS: usize = 8;
/// Relays the tracker follows.
pub const MAX_RELAYS: usize = 64;
/// Threads whose CPU state a block lists.
pub const MAX_THREADS: usize = 8;
/// Relay blocks printed per run (then one suppression line).
pub const MAX_BLOCKS: u32 = 32;
/// `PROGRESS_STARVED` summary lines printed per run (then one suppression line).
pub const MAX_STARVED_LINES: u32 = 64;
/// A worker may serve this many passes without polling before the count rule fires: four times [`crate::worker::LOOK_EVERY`], the
/// same `K = 16` the fairness model (`tests/relay_fairness.rs`) uses.
pub const BOUND_PASSES: u32 = 16;
/// Lines of one relay block, at most.
pub const MAX_BLOCK_LINES: usize = 1 + 1 + 3 + 4 + MAX_WORKERS + MAX_THREADS;
/// Named counters a block or a summary lists.
pub const MAX_COUNTERS: usize = 16;
/// Relays the summary context lists (the waiting, pending and contended ones first).
pub const MAX_SUMMARY_RELAYS: usize = 8;
/// Lines of the summary context, at most.
pub const MAX_CONTEXT_LINES: usize = MAX_SUMMARY_RELAYS + 3 + MAX_WORKERS;
/// Characters of one line, at most.
pub const MAX_LINE_CHARS: usize = 400;

/// What [`ENV`] says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    /// Unset, empty or `0`.
    Off,
    /// The threshold in milliseconds (`1..=`[`MAX_THRESHOLD_MS`]).
    On(u64),
    /// Present and not a decimal number in range: treated as off, said loudly once.
    Invalid,
}

/// Parse [`ENV`]'s value.
#[must_use]
pub fn parse_setting(value: Option<&str>) -> Setting {
    let Some(v) = value.filter(|v| !v.is_empty()) else {
        return Setting::Off;
    };
    if !v.bytes().all(|b| b.is_ascii_digit()) {
        return Setting::Invalid;
    }
    match v.parse::<u64>() {
        Ok(0) => Setting::Off,
        Ok(ms) if ms <= MAX_THRESHOLD_MS => Setting::On(ms),
        _ => Setting::Invalid,
    }
}

/// Whether a point must keep being visited by a BUSY worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Due {
    /// A worker that keeps serving must hit it at least once per threshold: its silence is a starvation.
    WhenBusy,
    /// Event-driven or idle-only: shown in the age table, never flagged by age alone.
    Informational,
}

/// The required visit points, derived from `worker::run_with_relay`, `ChanPlane::user_ring_serve` and `user_ring_peek_one`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Point {
    /// Every pass of the worker loop (`plane.worker_pass` returned).
    Scan,
    /// A pass that served at least one token (claim + serve).
    Serve,
    /// Any `poller.wait` of the worker (the busy look, an idle spin iteration, the park).
    Poll,
    /// The busy LOOK arm of `run_with_relay` (`AfterPass::Look`).
    Look,
    /// An idle spin iteration.
    SpinLook,
    /// A park wait ended (events or timeout).
    ParkWake,
    /// A ready tag at or above `OTHER_TAG_BASE` was dispatched (an engine event: the guest's completion interrupt).
    DispatchOther,
    /// A ready `COMPLETIONS_TAG` was dispatched.
    DispatchCompletions,
    /// The doorbell eventfd was drained.
    DoorbellDrain,
    /// `relay.peek(Peek::Event)`.
    PeekEvent,
    /// `relay.peek(Peek::Spin)`.
    PeekSpin,
    /// `relay.peek(Peek::Backstop)`.
    PeekBackstop,
    /// A USER-ring relay's turn ran (`ChanPlane::user_ring_serve`).
    RelayTurn,
    /// A look rang a relay's token (`user_ring_peek_one`).
    RelayRing,
}

/// Number of points.
pub const POINTS: usize = 14;

/// One row of the table.
#[derive(Debug, Clone, Copy)]
pub struct Def {
    /// The point.
    pub point: Point,
    /// Its name in the age table and in `PROGRESS_STARVED point=`.
    pub name: &'static str,
    /// Whether a busy worker must keep hitting it.
    pub due: Due,
    /// Where the hit is made (file and function), for the pin tests.
    pub site: &'static str,
}

/// THE table, in [`Point::index`] order.
pub const TABLE: [Def; POINTS] = [
    Def {
        point: Point::Scan,
        name: "scan",
        due: Due::WhenBusy,
        site: "kf-chan worker.rs run_with_relay: after plane.worker_pass (Tap::end_pass)",
    },
    Def {
        point: Point::Serve,
        name: "serve",
        due: Due::Informational,
        site: "kf-chan worker.rs run_with_relay: a pass with n > 0 (Tap::end_pass)",
    },
    Def {
        point: Point::Poll,
        name: "poll",
        due: Due::WhenBusy,
        site: "kf-chan worker.rs run_with_relay: every poller.wait (Tap::look, spin, park_end)",
    },
    Def {
        point: Point::Look,
        name: "look",
        due: Due::Informational,
        site: "kf-chan worker.rs run_with_relay: the AfterPass::Look arm (Tap::look)",
    },
    Def {
        point: Point::SpinLook,
        name: "spin_look",
        due: Due::Informational,
        site: "kf-chan worker.rs run_with_relay: each idle spin iteration (Tap::spin)",
    },
    Def {
        point: Point::ParkWake,
        name: "park_wake",
        due: Due::Informational,
        site: "kf-chan worker.rs run_with_relay: after the park wait (Tap::park_end)",
    },
    Def {
        point: Point::DispatchOther,
        name: "dispatch_other",
        due: Due::Informational,
        site: "kf-chan worker.rs run_with_relay: dispatch, tag >= OTHER_TAG_BASE (Tap::dispatch_other)",
    },
    Def {
        point: Point::DispatchCompletions,
        name: "dispatch_completions",
        due: Due::Informational,
        site: "kf-chan worker.rs run_with_relay: dispatch, COMPLETIONS_TAG (Tap::dispatch_completions)",
    },
    Def {
        point: Point::DoorbellDrain,
        name: "doorbell_drain",
        due: Due::Informational,
        site: "kf-chan worker.rs run_with_relay: dispatch, the worker eventfd drain (Tap::doorbell_drain)",
    },
    Def {
        point: Point::PeekEvent,
        name: "peek_event",
        due: Due::Informational,
        site: "kf-chan worker.rs Watched::peek(Peek::Event)",
    },
    Def {
        point: Point::PeekSpin,
        name: "peek_spin",
        due: Due::Informational,
        site: "kf-chan worker.rs Watched::peek(Peek::Spin)",
    },
    Def {
        point: Point::PeekBackstop,
        name: "peek_backstop",
        due: Due::Informational,
        site: "kf-chan worker.rs Watched::peek(Peek::Backstop)",
    },
    Def {
        point: Point::RelayTurn,
        name: "relay_turn",
        due: Due::Informational,
        site: "kf-qemu userpump.rs ChanPlane::user_ring_serve, before pump.turn",
    },
    Def {
        point: Point::RelayRing,
        name: "relay_ring",
        due: Due::Informational,
        site: "kf-qemu userpump.rs ChanPlane::user_ring_peek_one, the ring closure",
    },
];

impl Point {
    /// Position in [`TABLE`].
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }
    /// The table row.
    #[must_use]
    pub const fn def(self) -> &'static Def {
        &TABLE[self.index()]
    }
    /// The name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.def().name
    }
}

/// The board's start: [`now_ns`] counts from 1.
pub const START_NS: u64 = 1;

/// The monotonic clock of the board: ns since the first call, never 0.
#[must_use]
pub fn now_ns() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(std::time::Instant::now);
    u64::try_from(start.elapsed().as_nanos())
        .unwrap_or(u64::MAX - 1)
        .saturating_add(START_NS)
}

/// A worker's state word.
pub mod state {
    /// Not registered.
    pub const NONE: u8 = 0;
    /// In its loop (a pass, a look or a spin).
    pub const RUNNING: u8 = 1;
    /// In the park wait.
    pub const PARKED: u8 = 2;
}

/// One worker's cell (relaxed atomics, written only by that worker, read by the diagnostic thread).
#[derive(Debug)]
pub struct WorkerCell {
    /// Start of the last pass (ns, 0 = none yet).
    pub pass_ns: AtomicU64,
    /// Last `poller.wait` (ns).
    pub poll_ns: AtomicU64,
    /// Last busy look (ns).
    pub look_ns: AtomicU64,
    /// Passes begun.
    pub passes: AtomicU64,
    /// Busy looks.
    pub looks: AtomicU64,
    /// Polls (look, spin, park).
    pub polls: AtomicU64,
    /// Served passes since the last poll.
    pub since_poll: AtomicU32,
    /// A [`state`] value.
    pub state: AtomicU8,
    /// 0 = serving nothing, else `(1 << 63) | host_token`.
    pub serving: AtomicU64,
    /// Park waits entered.
    pub parks: AtomicU64,
    /// Last park activity (entered or left, ns; 0 = never parked).
    pub park_ns: AtomicU64,
}

/// The board: per-point last-hit timestamps and hit counters, plus the worker cells.
#[derive(Debug)]
pub struct Board {
    last: [AtomicU64; POINTS],
    hits: [AtomicU64; POINTS],
    workers: [WorkerCell; MAX_WORKERS],
    next_worker: AtomicUsize,
}

/// A point's reading.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PointSnap {
    /// Last hit (ns; 0 = never).
    pub last_ns: u64,
    /// Hits.
    pub hits: u64,
}

/// A worker's reading.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkerSnap {
    /// See [`WorkerCell`].
    pub pass_ns: u64,
    /// See [`WorkerCell`].
    pub poll_ns: u64,
    /// See [`WorkerCell`].
    pub look_ns: u64,
    /// See [`WorkerCell`].
    pub passes: u64,
    /// See [`WorkerCell`].
    pub looks: u64,
    /// See [`WorkerCell`].
    pub polls: u64,
    /// See [`WorkerCell`].
    pub since_poll: u32,
    /// See [`WorkerCell`].
    pub state: u8,
    /// See [`WorkerCell`].
    pub serving: u64,
    /// See [`WorkerCell`].
    pub parks: u64,
    /// See [`WorkerCell`].
    pub park_ns: u64,
}

/// The whole board at one instant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The reading's time.
    pub now_ns: u64,
    /// When the board started (the age of a never-hit point is counted from here).
    pub start_ns: u64,
    /// Per point, in [`Point::index`] order.
    pub points: [PointSnap; POINTS],
    /// Registered workers only.
    pub workers: Vec<WorkerSnap>,
}

/// A point's age.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Age {
    /// Never hit since the board started; the payload is the time since then.
    Never(u64),
    /// Hit; the payload is the time since the last hit.
    Since(u64),
}

impl Snapshot {
    /// The age of `p`.
    #[must_use]
    pub fn age(&self, p: Point) -> Age {
        let last = self.points[p.index()].last_ns;
        if last == 0 {
            Age::Never(self.now_ns.saturating_sub(self.start_ns))
        } else {
            Age::Since(self.now_ns.saturating_sub(last))
        }
    }
}

impl Board {
    /// An empty board.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            last: [const { AtomicU64::new(0) }; POINTS],
            hits: [const { AtomicU64::new(0) }; POINTS],
            workers: [const {
                WorkerCell {
                    pass_ns: AtomicU64::new(0),
                    poll_ns: AtomicU64::new(0),
                    look_ns: AtomicU64::new(0),
                    passes: AtomicU64::new(0),
                    looks: AtomicU64::new(0),
                    polls: AtomicU64::new(0),
                    since_poll: AtomicU32::new(0),
                    state: AtomicU8::new(0),
                    serving: AtomicU64::new(0),
                    parks: AtomicU64::new(0),
                    park_ns: AtomicU64::new(0),
                }
            }; MAX_WORKERS],
            next_worker: AtomicUsize::new(0),
        }
    }
    /// Stamp `p` at `now_ns` and count it.
    pub fn hit(&self, p: Point, now_ns: u64) {
        let i = p.index();
        self.last[i].fetch_max(now_ns.max(1), Ordering::Relaxed);
        self.hits[i].fetch_add(1, Ordering::Relaxed);
    }
    /// Register a worker; `None` past [`MAX_WORKERS`].
    pub fn register(&self) -> Option<usize> {
        let mut n = self.next_worker.load(Ordering::Relaxed);
        loop {
            if n >= MAX_WORKERS {
                return None;
            }
            match self.next_worker.compare_exchange_weak(
                n,
                n + 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    self.workers[n]
                        .state
                        .store(state::RUNNING, Ordering::Relaxed);
                    return Some(n);
                }
                Err(now) => n = now,
            }
        }
    }
    /// A worker begins a pass.
    pub fn begin_pass(&self, w: usize, now_ns: u64) {
        let Some(c) = self.workers.get(w) else {
            return;
        };
        c.pass_ns.store(now_ns.max(1), Ordering::Relaxed);
        c.passes.fetch_add(1, Ordering::Relaxed);
        c.state.store(state::RUNNING, Ordering::Relaxed);
    }
    /// A worker's pass returned having served `served` tokens.
    pub fn end_pass(&self, w: usize, served: usize, now_ns: u64) {
        self.hit(Point::Scan, now_ns);
        if served > 0 {
            self.hit(Point::Serve, now_ns);
            if let Some(c) = self.workers.get(w) {
                c.since_poll.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    /// A worker polled (`Point::Poll` and the per-worker word); `look` also stamps the busy look.
    pub fn poll(&self, w: usize, now_ns: u64, look: bool) {
        self.hit(Point::Poll, now_ns);
        if look {
            self.hit(Point::Look, now_ns);
        }
        let Some(c) = self.workers.get(w) else {
            return;
        };
        c.poll_ns.store(now_ns.max(1), Ordering::Relaxed);
        c.polls.fetch_add(1, Ordering::Relaxed);
        c.since_poll.store(0, Ordering::Relaxed);
        if look {
            c.look_ns.store(now_ns.max(1), Ordering::Relaxed);
            c.looks.fetch_add(1, Ordering::Relaxed);
        }
    }
    /// A worker entered (`true`) or left the park wait.
    pub fn park(&self, w: usize, parked: bool, now_ns: u64) {
        if !parked {
            self.hit(Point::ParkWake, now_ns);
        }
        if let Some(c) = self.workers.get(w) {
            if parked {
                c.parks.fetch_add(1, Ordering::Relaxed);
            }
            c.park_ns.store(now_ns.max(1), Ordering::Relaxed);
            let s = if parked {
                state::PARKED
            } else {
                state::RUNNING
            };
            c.state.store(s, Ordering::Relaxed);
        }
    }
    /// What a worker is serving.
    pub fn serving(&self, w: usize, token: Option<u32>) {
        if let Some(c) = self.workers.get(w) {
            let v = token.map_or(0, |t| (1 << 63) | u64::from(t));
            c.serving.store(v, Ordering::Relaxed);
        }
    }
    /// Read everything.
    #[must_use]
    pub fn snapshot(&self, now_ns: u64, start_ns: u64) -> Snapshot {
        let g = |a: &AtomicU64| a.load(Ordering::Relaxed);
        let mut points = [PointSnap::default(); POINTS];
        for (i, p) in points.iter_mut().enumerate() {
            *p = PointSnap {
                last_ns: g(&self.last[i]),
                hits: g(&self.hits[i]),
            };
        }
        let n = self.next_worker.load(Ordering::Relaxed).min(MAX_WORKERS);
        let workers = self.workers[..n]
            .iter()
            .map(|c| WorkerSnap {
                pass_ns: g(&c.pass_ns),
                poll_ns: g(&c.poll_ns),
                look_ns: g(&c.look_ns),
                passes: g(&c.passes),
                looks: g(&c.looks),
                polls: g(&c.polls),
                since_poll: c.since_poll.load(Ordering::Relaxed),
                state: c.state.load(Ordering::Relaxed),
                serving: g(&c.serving),
                parks: g(&c.parks),
                park_ns: g(&c.park_ns),
            })
            .collect();
        Snapshot {
            now_ns,
            start_ns,
            points,
            workers,
        }
    }
}

impl Default for Board {
    fn default() -> Self {
        Self::new()
    }
}

/// The process-wide board (one device per process).
pub static BOARD: Board = Board::new();

static THRESHOLD_MS: AtomicU64 = AtomicU64::new(0);
static SETTING: std::sync::OnceLock<Setting> = std::sync::OnceLock::new();

/// Read [`ENV`] once; idempotent. Returns the setting in force.
pub fn init_from_env() -> Setting {
    *SETTING.get_or_init(|| {
        let v = std::env::var(ENV).ok();
        let s = parse_setting(v.as_deref());
        if let Setting::On(ms) = s {
            THRESHOLD_MS.store(ms, Ordering::Relaxed);
            let _ = now_ns(); // the board's clock starts now
        }
        s
    })
}

/// The threshold in ms in force (0 = off): a relaxed load.
#[must_use]
pub fn threshold_ms() -> u64 {
    THRESHOLD_MS.load(Ordering::Relaxed)
}

/// Whether the diagnostic is on.
#[must_use]
pub fn on() -> bool {
    threshold_ms() != 0
}

thread_local! {
    /// The calling thread's worker cell, once registered.
    static WORKER_SLOT: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// What the worker loop holds when the diagnostic is on.
#[derive(Debug)]
pub struct Tap {
    w: usize,
}

impl Tap {
    /// Register the calling thread as a worker; `None` with the diagnostic off or past [`MAX_WORKERS`].
    #[must_use]
    pub fn register() -> Option<Self> {
        if !on() {
            return None;
        }
        let w = BOARD.register()?;
        WORKER_SLOT.with(|c| c.set(Some(w)));
        Some(Self { w })
    }
    /// The worker's cell index.
    #[must_use]
    pub fn worker(&self) -> usize {
        self.w
    }
    /// See [`Board::begin_pass`].
    pub fn begin_pass(&self) {
        BOARD.begin_pass(self.w, now_ns());
    }
    /// See [`Board::end_pass`].
    pub fn end_pass(&self, served: usize) {
        BOARD.end_pass(self.w, served, now_ns());
    }
    /// A busy look began.
    pub fn look(&self) {
        let now = now_ns();
        BOARD.poll(self.w, now, true);
    }
    /// An idle spin iteration.
    pub fn spin(&self) {
        let now = now_ns();
        BOARD.poll(self.w, now, false);
        BOARD.hit(Point::SpinLook, now);
    }
    /// The park wait is about to start.
    pub fn park_begin(&self) {
        BOARD.park(self.w, true, now_ns());
    }
    /// The park wait ended.
    pub fn park_end(&self) {
        let now = now_ns();
        BOARD.park(self.w, false, now);
        BOARD.poll(self.w, now, false);
    }
    /// A tag was dispatched: engine event.
    pub fn dispatch_other(&self) {
        BOARD.hit(Point::DispatchOther, now_ns());
    }
    /// A tag was dispatched: completions.
    pub fn dispatch_completions(&self) {
        BOARD.hit(Point::DispatchCompletions, now_ns());
    }
    /// The doorbell eventfd was drained.
    pub fn doorbell_drain(&self) {
        BOARD.hit(Point::DoorbellDrain, now_ns());
    }
    /// `relay.peek(why)` is about to run.
    pub fn peek(&self, why: Peek) {
        let p = match why {
            Peek::Event => Point::PeekEvent,
            Peek::Spin => Point::PeekSpin,
            Peek::Backstop => Point::PeekBackstop,
        };
        BOARD.hit(p, now_ns());
    }
}

/// Hit `p` from the calling worker thread's context (the relay path in `kf-qemu`); nothing when the diagnostic is off.
pub fn hit_point(p: Point) {
    if on() {
        BOARD.hit(p, now_ns());
    }
}

/// Record (`Some`) or clear (`None`) the host token the calling worker is serving; nothing when the diagnostic is off or the thread is no worker.
pub fn set_serving(token: Option<u32>) {
    if !on() {
        return;
    }
    WORKER_SLOT.with(|c| {
        if let Some(w) = c.get() {
            BOARD.serving(w, token);
        }
    });
}

/// A violation the monitor found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Starved {
    /// A [`Due::WhenBusy`] point silent for more than the threshold while workers kept serving.
    Point {
        /// The point.
        point: Point,
        /// Time since its last hit (or since the board started).
        age_ns: u64,
        /// Passes that served something in the window.
        serve_delta: u64,
    },
    /// A worker served more than [`BOUND_PASSES`] passes without a poll.
    WorkerPoll {
        /// Worker cell.
        w: usize,
        /// Served passes since the last poll.
        passes: u32,
        /// Time since its last poll.
        age_ns: u64,
    },
    /// A worker has been inside one pass (not parked) for more than the threshold.
    WorkerStuck {
        /// Worker cell.
        w: usize,
        /// Time since the pass began.
        age_ns: u64,
        /// What it was serving (see [`WorkerCell::serving`]).
        serving: u64,
    },
}

/// The window/rate logic of the summary line.
#[derive(Debug, Clone)]
pub struct Monitor {
    threshold_ns: u64,
    window_start_ns: u64,
    serve_at_start: u64,
    emitted_poll: [u64; MAX_WORKERS],
    emitted_stuck: [u64; MAX_WORKERS],
}

impl Monitor {
    /// A monitor whose first window starts at `snap` (hits counted from there).
    #[must_use]
    pub fn new(threshold_ms: u64, snap: &Snapshot) -> Self {
        Self {
            threshold_ns: threshold_ms.saturating_mul(1_000_000),
            window_start_ns: snap.now_ns,
            serve_at_start: snap.points[Point::Serve.index()].hits,
            emitted_poll: [0; MAX_WORKERS],
            emitted_stuck: [0; MAX_WORKERS],
        }
    }
    /// Evaluate `snap`: point violations once per window, worker violations at most once per threshold each.
    pub fn evaluate(&mut self, snap: &Snapshot) -> Vec<Starved> {
        let now = snap.now_ns;
        let t = self.threshold_ns;
        let mut out = Vec::new();
        // the rate limit: a worker rule repeats at most once per threshold
        let due = |last: &mut u64| {
            if *last == 0 || now.saturating_sub(*last) >= t {
                *last = now;
                true
            } else {
                false
            }
        };
        for (w, ws) in snap.workers.iter().enumerate().take(MAX_WORKERS) {
            if ws.since_poll > BOUND_PASSES && due(&mut self.emitted_poll[w]) {
                out.push(Starved::WorkerPoll {
                    w,
                    passes: ws.since_poll,
                    age_ns: now.saturating_sub(ws.poll_ns),
                });
            }
            let stuck =
                ws.state == state::RUNNING && ws.pass_ns != 0 && now.saturating_sub(ws.pass_ns) > t;
            if stuck && due(&mut self.emitted_stuck[w]) {
                out.push(Starved::WorkerStuck {
                    w,
                    age_ns: now.saturating_sub(ws.pass_ns),
                    serving: ws.serving,
                });
            }
        }
        if now.saturating_sub(self.window_start_ns) >= t {
            let serve = snap.points[Point::Serve.index()].hits;
            let serve_delta = serve.saturating_sub(self.serve_at_start);
            if serve_delta > 0 {
                for d in TABLE.iter().filter(|d| d.due == Due::WhenBusy) {
                    let age_ns = match snap.age(d.point) {
                        Age::Never(a) | Age::Since(a) => a,
                    };
                    if age_ns > t {
                        out.push(Starved::Point {
                            point: d.point,
                            age_ns,
                            serve_delta,
                        });
                    }
                }
            }
            self.window_start_ns = now;
            self.serve_at_start = serve;
        }
        out
    }
}

/// What a relay waits for, without the payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WaitKind {
    /// `Wait::Marker`.
    Marker,
    /// `Wait::Popped`.
    Popped,
    /// `Wait::Contended`.
    Contended,
}

impl WaitKind {
    /// The name in the block.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Marker => "marker",
            Self::Popped => "popped",
            Self::Contended => "contended",
        }
    }
}

/// A wait, as the tracker keys it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WaitKey {
    /// Kind.
    pub kind: WaitKind,
    /// The target count (0 for contended).
    pub target: u64,
}

impl WaitKey {
    /// From the relay's [`Wait`].
    #[must_use]
    pub const fn of(w: Wait) -> Self {
        match w {
            Wait::Marker { target } => Self {
                kind: WaitKind::Marker,
                target,
            },
            Wait::Popped { target } => Self {
                kind: WaitKind::Popped,
                target,
            },
            Wait::Contended => Self {
                kind: WaitKind::Contended,
                target: 0,
            },
        }
    }
}

/// The start of a satisfied-but-unpeeked interval: the later of the first time it was seen true and the last look.
#[must_use]
pub const fn interval_start(first_true_ns: u64, last_peek_ns: u64) -> u64 {
    if first_true_ns > last_peek_ns {
        first_true_ns
    } else {
        last_peek_ns
    }
}

/// Tracks, per relay, how long its wait has been satisfied while no look evaluated it.
#[derive(Debug, Default)]
pub struct Tracker {
    slots: Vec<Slot>,
}

#[derive(Debug, Clone, Copy)]
struct Slot {
    id: u64,
    key: Option<WaitKey>,
    since: Option<u64>,
    reported: bool,
}

impl Tracker {
    /// An empty tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// One observation of relay `id` at `now_ns`. `wait` is its recorded wait (`None` = none), `satisfied` whether
    /// `Wait::satisfied(marks, popped)` holds on the passive reads, `last_peek_ns` the relay's last look/turn stamp (0 = never).
    /// Returns the age (ns) of the satisfied-unpeeked interval the FIRST time it exceeds `threshold_ns`; a peek, a changed wait or an
    /// unsatisfied reading re-arms it.
    pub fn observe(
        &mut self,
        now_ns: u64,
        id: u64,
        wait: Option<WaitKey>,
        satisfied: bool,
        last_peek_ns: u64,
        threshold_ns: u64,
    ) -> Option<u64> {
        let idx = if let Some(i) = self.slots.iter().position(|s| s.id == id) {
            i
        } else {
            if self.slots.len() >= MAX_RELAYS {
                return None;
            }
            self.slots.push(Slot {
                id,
                key: None,
                since: None,
                reported: false,
            });
            self.slots.len() - 1
        };
        let slot = &mut self.slots[idx];
        let Some(key) = wait.filter(|_| satisfied) else {
            slot.key = wait;
            slot.since = None;
            slot.reported = false;
            return None;
        };
        if slot.key != Some(key) {
            slot.key = Some(key);
            slot.since = None;
            slot.reported = false;
        }
        let peek = last_peek_ns.min(now_ns);
        let since = match slot.since {
            None => interval_start(now_ns, peek),
            Some(s) if peek > s => {
                slot.reported = false; // a look re-arms the condition
                peek
            }
            Some(s) => s,
        };
        slot.since = Some(since);
        let age = now_ns.saturating_sub(since);
        if age > threshold_ns && !slot.reported {
            slot.reported = true;
            Some(age)
        } else {
            None
        }
    }
    /// The current age of `id`'s interval, if one is open (for the model comparison).
    #[must_use]
    pub fn open_age_ns(&self, id: u64, now_ns: u64) -> Option<u64> {
        self.slots
            .iter()
            .find(|s| s.id == id)
            .and_then(|s| s.since)
            .map(|since| now_ns.saturating_sub(since))
    }
    /// Forget every relay not in `live`.
    pub fn retain(&mut self, live: &[u64]) {
        self.slots.retain(|s| live.contains(&s.id));
    }
    /// Relays followed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.len()
    }
    /// No relay followed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

/// A thread's CPU state from `/proc/self/task/<tid>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadCpu {
    /// Thread id.
    pub tid: u64,
    /// `comm`, at most 15 bytes.
    pub comm: String,
    /// The state letter (`R`, `S`, `D`, ...).
    pub state: char,
    /// User ticks.
    pub utime: u64,
    /// System ticks.
    pub stime: u64,
}

/// Parse a `/proc/<pid>/task/<tid>/stat` line.
#[must_use]
pub fn parse_thread_stat(tid: u64, stat: &str) -> Option<ThreadCpu> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    if close < open {
        return None;
    }
    let comm: String = stat[open + 1..close].chars().take(15).collect();
    let rest: Vec<&str> = stat[close + 1..].split_whitespace().collect();
    // after the name: state, ppid, pgrp, session, tty, tpgid, flags, minflt, cminflt, majflt, cmajflt, utime, stime
    if rest.len() < 13 {
        return None;
    }
    Some(ThreadCpu {
        tid,
        comm,
        state: rest[0].chars().next()?,
        utime: rest[11].parse().ok()?,
        stime: rest[12].parse().ok()?,
    })
}

/// What a relay block reports about one relay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayFacts {
    /// Guest `(client, object)`.
    pub key: (u32, u32),
    /// Host token.
    pub host: u32,
    /// The wait.
    pub wait: WaitKey,
    /// Markers processed (the tagged `USERD.REFERENCE`, `Marks::peek`).
    pub marks: u64,
    /// Entries popped (`USERD.GP_GET`, `Fetch::peek`).
    pub popped: u64,
    /// ms the wait has been satisfied and unpeeked.
    pub unpeeked_ms: u64,
    /// ms since the relay's last look/turn (`None` = never).
    pub last_peek_age_ms: Option<u64>,
    /// The threshold in ms.
    pub threshold_ms: u64,
    /// The relay's contention and demand state (hypothesis H-64: after `CONTENTION_RETRIES` lost turns only the backstop re-rings).
    pub brief: RelayBrief,
    /// Worker-level counters, `(name, value)`, at most [`MAX_COUNTERS`] are listed.
    pub counters: Vec<(&'static str, u64)>,
}

/// One relay's contention, demand and lifecycle flags, read without a lock where the record has an atomic (all of them but the
/// twin's `stopped` / `disabled`, which need the plane's table and are `None` when it was busy).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayBrief {
    /// Guest `(client, object)`.
    pub key: (u32, u32),
    /// Host token.
    pub host: u32,
    /// The recorded wait, if any.
    pub wait: Option<WaitKey>,
    /// Consecutive contended turns (`Cell::contended_run`).
    pub contended_run: u32,
    /// `Cell::retry` (the durable demand flag, raw).
    pub retry: bool,
    /// `Cell::pending()` (`retry` and not canceled): what a look treats as demand.
    pub pending: bool,
    /// `Cell::is_waiting()` (the wait mirror a look reads).
    pub waiting: bool,
    /// `Cell::is_canceled()`.
    pub canceled: bool,
    /// The twin's `stopped` (`None` = the table was busy).
    pub stopped: Option<bool>,
    /// The twin's `disabled` (`None` = the table was busy).
    pub disabled: Option<bool>,
    /// `Inbox::walk_busy` flag.
    pub walk_busy: bool,
    /// Tokens waiting for a walk (`None` = the list was busy).
    pub walk_waiters: Option<u64>,
}

fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

fn maybe(b: Option<bool>) -> &'static str {
    b.map_or("unknown", yes_no)
}

const fn ns_to_ms(ns: u64) -> u64 {
    ns / 1_000_000
}

fn serving_text(serving: u64) -> String {
    if serving == 0 {
        "none".to_string()
    } else {
        format!("{:#x}", serving & !(1 << 63))
    }
}

fn age_text(a: Age) -> String {
    match a {
        Age::Never(_) => "never".to_string(),
        Age::Since(ns) => format!("{:#x}", ns_to_ms(ns)),
    }
}

/// `since` as an age against `now` (`never` for a stamp of 0).
fn stamp_age(stamp_ns: u64, now_ns: u64) -> Age {
    if stamp_ns == 0 {
        Age::Never(0)
    } else {
        Age::Since(now_ns.saturating_sub(stamp_ns))
    }
}

fn budget(used: &mut u32, max: u32) -> Admit {
    if *used < max {
        *used += 1;
        Admit::Yes
    } else if *used == max {
        *used = used.saturating_add(1);
        Admit::Suppress
    } else {
        Admit::No
    }
}

/// The age table of ALL points, four per line, the board's time first.
fn render_points(snap: &Snapshot) -> Vec<String> {
    let entries: Vec<String> = TABLE
        .iter()
        .map(|d| {
            format!(
                "{}={}/{:#x}",
                d.name,
                age_text(snap.age(d.point)),
                snap.points[d.point.index()].hits
            )
        })
        .collect();
    entries
        .chunks(4)
        .enumerate()
        .map(|(i, chunk)| {
            let mut l = String::from("kf3: PROGRESS_POINTS");
            if i == 0 {
                l.push_str(&format!(
                    " now_ms={:#x} (name=age_ms/hits)",
                    ns_to_ms(snap.now_ns)
                ));
            }
            for e in chunk {
                l.push(' ');
                l.push_str(e);
            }
            l
        })
        .collect()
}

/// Every registered worker, one line each.
fn render_workers(snap: &Snapshot) -> Vec<String> {
    snap.workers
        .iter()
        .take(MAX_WORKERS)
        .enumerate()
        .map(|(i, w)| {
            let state = match w.state {
                state::RUNNING => "running",
                state::PARKED => "parked",
                _ => "none",
            };
            format!(
                "kf3: PROGRESS_WORKER w={i:#x} state={state} serving={} passes_since_poll={:#x} poll_age_ms={} look_age_ms={} parks={:#x} park_age_ms={} passes={:#x} polls={:#x} looks={:#x}",
                serving_text(w.serving),
                w.since_poll,
                age_text(stamp_age(w.poll_ns, snap.now_ns)),
                age_text(stamp_age(w.look_ns, snap.now_ns)),
                w.parks,
                age_text(stamp_age(w.park_ns, snap.now_ns)),
                w.passes,
                w.polls,
                w.looks
            )
        })
        .collect()
}

/// Render one relay's state line (H-64: `h64_backstop_only=yes` when only the safety net may still re-ring it).
#[must_use]
pub fn render_relay_state(b: &RelayBrief) -> String {
    let wait = b.wait.map_or("none", |w| w.kind.name());
    let stranded =
        (b.pending || b.waiting) && b.contended_run >= crate::user_loop::CONTENTION_RETRIES;
    format!(
        "kf3: PROGRESS_RELAY_STATE relay={:#x}:{:#x} host={:#x} wait={wait} contended_run={:#x} retry_budget={:#x} h64_backstop_only={} retry_flag={} pending={} waiting_flag={} canceled={} stopped={} disabled={} walk_busy={} walk_waiters={}",
        b.key.0,
        b.key.1,
        b.host,
        b.contended_run,
        crate::user_loop::CONTENTION_RETRIES,
        yes_no(stranded),
        yes_no(b.retry),
        yes_no(b.pending),
        yes_no(b.waiting),
        yes_no(b.canceled),
        maybe(b.stopped),
        maybe(b.disabled),
        yes_no(b.walk_busy),
        b.walk_waiters
            .map_or_else(|| "unknown".to_string(), |n| format!("{n:#x}")),
    )
}

/// Render named counters, six per line, at most [`MAX_COUNTERS`] of them.
#[must_use]
pub fn render_counters(c: &[(&'static str, u64)]) -> Vec<String> {
    c.iter()
        .take(MAX_COUNTERS)
        .collect::<Vec<_>>()
        .chunks(6)
        .map(|chunk| {
            let mut l = String::from("kf3: PROGRESS_COUNTERS");
            for (name, v) in chunk {
                l.push_str(&format!(" {name}={v:#x}"));
            }
            l
        })
        .collect()
}

/// The per-second summary's context: up to [`MAX_SUMMARY_RELAYS`] relay state lines (waiting, pending and contended first), the
/// counters and every worker. Hex-only, at most [`MAX_CONTEXT_LINES`] lines.
#[must_use]
pub fn render_summary_context(
    snap: &Snapshot,
    counters: &[(&'static str, u64)],
    relays: &[RelayBrief],
) -> Vec<String> {
    let rank = |b: &RelayBrief| {
        if (b.pending || b.waiting) && b.contended_run >= crate::user_loop::CONTENTION_RETRIES {
            0
        } else if b.pending || b.waiting || b.contended_run > 0 {
            1
        } else {
            2
        }
    };
    let mut order: Vec<&RelayBrief> = relays.iter().collect();
    order.sort_by_key(|b| rank(b));
    let mut out: Vec<String> = order
        .into_iter()
        .take(MAX_SUMMARY_RELAYS)
        .map(render_relay_state)
        .collect();
    out.extend(render_counters(counters));
    out.extend(render_workers(snap));
    out
}

/// Render one relay's block: the relay line, the age table of ALL points, every worker, the listed threads. Hex-only, bounded.
#[must_use]
pub fn render_relay_block(f: &RelayFacts, snap: &Snapshot, threads: &[ThreadCpu]) -> Vec<String> {
    let mut out = Vec::with_capacity(MAX_BLOCK_LINES);
    out.push(format!(
        "kf3: PROGRESS_STARVED_RELAY relay={:#x}:{:#x} host={:#x} wait={} target={:#x} marks={:#x} popped={:#x} satisfied_unpeeked_ms={:#x} last_peek_age_ms={} threshold_ms={:#x}",
        f.key.0,
        f.key.1,
        f.host,
        f.wait.kind.name(),
        f.wait.target,
        f.marks,
        f.popped,
        f.unpeeked_ms,
        f.last_peek_age_ms
            .map_or_else(|| "never".to_string(), |a| format!("{a:#x}")),
        f.threshold_ms,
    ));
    out.push(render_relay_state(&f.brief));
    out.extend(render_counters(&f.counters));
    out.extend(render_points(snap));
    out.extend(render_workers(snap));
    for t in threads.iter().take(MAX_THREADS) {
        out.push(format!(
            "kf3: PROGRESS_THREAD tid={:#x} comm={} state={} utime_ticks={:#x} stime_ticks={:#x}",
            t.tid,
            t.comm
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                })
                .collect::<String>(),
            t.state,
            t.utime,
            t.stime
        ));
    }
    out
}

/// Render one [`Starved`] as its single `PROGRESS_STARVED` line.
#[must_use]
pub fn render_starved(s: &Starved, threshold_ms: u64) -> String {
    let t = threshold_ms;
    match *s {
        Starved::Point {
            point,
            age_ns,
            serve_delta,
        } => format!(
            "kf3: PROGRESS_STARVED point={} age_ms={:#x} serve_delta={:#x} threshold_ms={t:#x}",
            point.name(),
            ns_to_ms(age_ns),
            serve_delta
        ),
        Starved::WorkerPoll { w, passes, age_ns } => format!(
            "kf3: PROGRESS_STARVED point=worker_poll age_ms={:#x} w={w:#x} passes_since_poll={passes:#x} bound={BOUND_PASSES:#x}",
            ns_to_ms(age_ns)
        ),
        Starved::WorkerStuck { w, age_ns, serving } => format!(
            "kf3: PROGRESS_STARVED point=worker_stuck age_ms={:#x} w={w:#x} serving={}",
            ns_to_ms(age_ns),
            serving_text(serving)
        ),
    }
}

/// Output budget of the diagnostic thread.
#[derive(Debug, Default)]
pub struct Printer {
    blocks: u32,
    starved: u32,
}

/// What the printer allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admit {
    /// Print it.
    Yes,
    /// Print the one suppression line (this is the first refusal).
    Suppress,
    /// Print nothing.
    No,
}

impl Printer {
    /// Admit a relay block?
    pub fn block(&mut self) -> Admit {
        budget(&mut self.blocks, MAX_BLOCKS)
    }
    /// Admit a `PROGRESS_STARVED` summary line?
    pub fn starved(&mut self) -> Admit {
        budget(&mut self.starved, MAX_STARVED_LINES)
    }
}

/// The suppression line.
#[must_use]
pub fn suppression_line(what: &str) -> String {
    let limit = if what == "blocks" {
        MAX_BLOCKS
    } else {
        MAX_STARVED_LINES
    };
    format!(
        "kf3: PROGRESS_SUPPRESSED {what}: the output budget of this run is spent limit={limit:#x}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u64 = 1_000_000;

    // ---- the switch

    #[test]
    fn the_switch_is_a_decimal_number_of_milliseconds_and_off_by_default() {
        for off in [None, Some(""), Some("0")] {
            assert_eq!(parse_setting(off), Setting::Off, "{off:?}");
        }
        assert_eq!(parse_setting(Some("1")), Setting::On(1));
        assert_eq!(parse_setting(Some("1000")), Setting::On(1000));
        assert_eq!(
            parse_setting(Some("3600000")),
            Setting::On(MAX_THRESHOLD_MS)
        );
    }

    #[test]
    fn a_malformed_or_out_of_range_switch_is_invalid_never_silently_on() {
        for bad in [
            "3600001",
            "-1",
            "1.5",
            "0x10",
            " 5",
            "5 ",
            "abc",
            "1e3",
            "+5",
            "00x",
            "99999999999999999999999",
        ] {
            assert_eq!(parse_setting(Some(bad)), Setting::Invalid, "{bad:?}");
        }
    }

    // ---- the table

    /// The documented list, in order. A new point is a deliberate edit of this list AND of the design doc.
    const NAMES: [&str; POINTS] = [
        "scan",
        "serve",
        "poll",
        "look",
        "spin_look",
        "park_wake",
        "dispatch_other",
        "dispatch_completions",
        "doorbell_drain",
        "peek_event",
        "peek_spin",
        "peek_backstop",
        "relay_turn",
        "relay_ring",
    ];

    #[test]
    fn the_table_is_the_documented_list_in_index_order() {
        for (i, d) in TABLE.iter().enumerate() {
            assert_eq!(d.point.index(), i, "{}", d.name);
            assert_eq!(d.name, NAMES[i]);
            assert!(!d.site.is_empty(), "{} has no site", d.name);
        }
    }

    #[test]
    fn point_names_are_unique_lowercase_words() {
        let mut seen = std::collections::HashSet::new();
        for d in &TABLE {
            assert!(d.name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'));
            assert!(seen.insert(d.name), "{} twice", d.name);
        }
    }

    #[test]
    fn only_scan_and_poll_are_flagged_by_age_everything_else_is_informational() {
        for d in &TABLE {
            let want = matches!(d.point, Point::Scan | Point::Poll);
            assert_eq!(d.due == Due::WhenBusy, want, "{}", d.name);
        }
    }

    // ---- the board

    #[test]
    fn a_hit_stamps_and_counts_and_never_stamps_zero() {
        let b = Board::new();
        b.hit(Point::Poll, 0);
        b.hit(Point::Poll, 500);
        b.hit(Point::Look, 7);
        let s = b.snapshot(1000, 1);
        assert_eq!(s.points[Point::Poll.index()].hits, 2);
        assert_eq!(s.points[Point::Poll.index()].last_ns, 500);
        assert_eq!(s.points[Point::Look.index()].last_ns, 7);
        assert_eq!(s.points[Point::Scan.index()], PointSnap::default());
        let c = Board::new();
        c.hit(Point::Scan, 0);
        assert_eq!(
            c.snapshot(5, 1).points[0].last_ns,
            1,
            "0 means never, so a hit at 0 is 1"
        );
    }

    #[test]
    fn a_hit_never_moves_a_stamp_backwards() {
        let b = Board::new();
        b.hit(Point::Scan, 900);
        b.hit(Point::Scan, 100);
        assert_eq!(b.snapshot(1000, 1).points[0].last_ns, 900);
    }

    #[test]
    fn ages_are_since_the_last_hit_or_since_the_start_when_never_hit() {
        let b = Board::new();
        b.hit(Point::Look, 400);
        let s = b.snapshot(1000, 100);
        assert_eq!(s.age(Point::Look), Age::Since(600));
        assert_eq!(s.age(Point::Poll), Age::Never(900));
        let late = b.snapshot(100, 100);
        assert_eq!(
            late.age(Point::Look),
            Age::Since(0),
            "a stamp in the future saturates"
        );
    }

    #[test]
    fn workers_register_up_to_the_cap_and_only_registered_ones_are_read() {
        let b = Board::new();
        for i in 0..MAX_WORKERS {
            assert_eq!(b.register(), Some(i));
        }
        assert_eq!(b.register(), None);
        assert_eq!(b.snapshot(10, 1).workers.len(), MAX_WORKERS);
        let c = Board::new();
        assert_eq!(c.register(), Some(0));
        assert_eq!(c.snapshot(10, 1).workers.len(), 1);
    }

    #[test]
    fn a_pass_counts_passes_since_the_last_poll_and_a_poll_resets_it() {
        let b = Board::new();
        let w = b.register().unwrap();
        for i in 0..5 {
            b.begin_pass(w, 10 + i);
            b.end_pass(w, 2, 11 + i);
        }
        b.begin_pass(w, 30);
        b.end_pass(w, 0, 31); // an empty pass does not count: only SERVED passes starve the world
        let s = b.snapshot(40, 1);
        assert_eq!(s.workers[0].since_poll, 5);
        assert_eq!(s.workers[0].passes, 6);
        assert_eq!(s.points[Point::Scan.index()].hits, 6);
        assert_eq!(s.points[Point::Serve.index()].hits, 5);
        b.poll(w, 50, true);
        let s = b.snapshot(60, 1);
        assert_eq!(s.workers[0].since_poll, 0);
        assert_eq!(s.workers[0].polls, 1);
        assert_eq!(s.workers[0].looks, 1);
        assert_eq!(s.workers[0].look_ns, 50);
        assert_eq!(s.points[Point::Poll.index()].hits, 1);
        b.poll(w, 70, false);
        let s = b.snapshot(80, 1);
        assert_eq!((s.workers[0].polls, s.workers[0].looks), (2, 1));
        assert_eq!(s.workers[0].poll_ns, 70);
        assert_eq!(s.workers[0].look_ns, 50, "a plain poll is not a look");
    }

    #[test]
    fn park_and_serving_are_per_worker_state() {
        let b = Board::new();
        let w = b.register().unwrap();
        b.park(w, true, 5);
        assert_eq!(b.snapshot(9, 1).workers[0].state, state::PARKED);
        b.park(w, false, 6);
        assert_eq!(b.snapshot(9, 1).workers[0].state, state::RUNNING);
        b.serving(w, Some(0x3f));
        assert_eq!(b.snapshot(9, 1).workers[0].serving, (1 << 63) | 0x3f);
        b.serving(w, None);
        assert_eq!(b.snapshot(9, 1).workers[0].serving, 0);
        b.serving(w + 100, Some(1)); // out-of-range worker: ignored, no panic
    }

    // ---- the monitor

    fn snap(now: u64, f: impl FnOnce(&Board)) -> Snapshot {
        let b = Board::new();
        let w = b.register().unwrap();
        b.park(w, false, 1);
        f(&b);
        b.snapshot(now, 1)
    }

    #[test]
    fn a_busy_worker_that_never_polls_is_reported_once_per_window() {
        let thr = 1000;
        let b = Board::new();
        let w = b.register().unwrap();
        b.hit(Point::Poll, 1);
        let mut m = Monitor::new(thr, &b.snapshot(1, 1));
        let mut t = 1;
        let mut found = Vec::new();
        // the worker keeps serving; nothing polls
        for _ in 0..40 {
            t += 100 * MS;
            b.begin_pass(w, t);
            b.end_pass(w, 1, t + 1);
            found.extend(m.evaluate(&b.snapshot(t + 2, 1)));
        }
        let pts: Vec<_> = found
            .iter()
            .filter(|s| {
                matches!(
                    s,
                    Starved::Point {
                        point: Point::Poll,
                        ..
                    }
                )
            })
            .collect();
        // 40 * 100 ms = 4 s of silence: one report per 1 s window after the first full window, never one per evaluation
        assert!(
            (2..=4).contains(&pts.len()),
            "{} reports: {found:?}",
            pts.len()
        );
        if let Starved::Point {
            serve_delta,
            age_ns,
            ..
        } = pts[0]
        {
            assert!(*serve_delta > 0);
            assert!(*age_ns > thr * MS);
        }
    }

    #[test]
    fn an_idle_system_is_never_starved_whatever_the_ages() {
        let b = Board::new();
        let _ = b.register();
        let mut m = Monitor::new(1000, &b.snapshot(1, 1));
        for k in 1..10 {
            // nothing served: nothing is due
            assert!(m.evaluate(&b.snapshot(k * 1000 * MS, 1)).is_empty());
        }
    }

    #[test]
    fn a_polling_busy_worker_is_not_starved() {
        let b = Board::new();
        let w = b.register().unwrap();
        let mut m = Monitor::new(1000, &b.snapshot(1, 1));
        let mut t = 1;
        for _ in 0..100 {
            t += 100 * MS;
            b.begin_pass(w, t);
            b.end_pass(w, 1, t + 1);
            b.poll(w, t + 2, true);
            assert!(m.evaluate(&b.snapshot(t + 3, 1)).is_empty());
        }
    }

    #[test]
    fn the_age_boundary_is_strictly_greater_than_the_threshold() {
        let thr = 1000;
        let b = Board::new();
        let w = b.register().unwrap();
        b.hit(Point::Poll, 10);
        let mut m = Monitor::new(thr, &b.snapshot(10, 10));
        b.begin_pass(w, 20);
        b.end_pass(w, 1, 21);
        // the window closes at 10 + thr ms; the age of Poll is exactly thr ms there
        let at = 10 + thr * MS;
        assert!(
            m.evaluate(&b.snapshot(at, 10)).iter().all(|s| !matches!(
                s,
                Starved::Point {
                    point: Point::Poll,
                    ..
                }
            )),
            "age == threshold is not starved"
        );
        b.begin_pass(w, at);
        b.end_pass(w, 1, at + 1);
        let found = m.evaluate(&b.snapshot(at + thr * MS, 10));
        assert!(
            found.iter().any(|s| matches!(
                s,
                Starved::Point {
                    point: Point::Poll,
                    ..
                }
            )),
            "{found:?}"
        );
    }

    #[test]
    fn a_never_hit_poll_counts_from_the_start_of_the_board() {
        let b = Board::new();
        let w = b.register().unwrap();
        let mut m = Monitor::new(100, &b.snapshot(1, 1));
        b.begin_pass(w, 5);
        b.end_pass(w, 3, 6);
        let found = m.evaluate(&b.snapshot(1 + 150 * MS, 1));
        assert!(
            found.iter().any(|s| matches!(
                s,
                Starved::Point {
                    point: Point::Poll,
                    ..
                }
            )),
            "{found:?}"
        );
    }

    #[test]
    fn the_per_worker_bound_fires_beyond_the_bound_not_at_it_and_is_rate_limited() {
        let b = Board::new();
        let w = b.register().unwrap();
        let thr_ms = 1000;
        let mut m = Monitor::new(thr_ms, &b.snapshot(1, 1));
        let mut t = 1;
        for _ in 0..BOUND_PASSES {
            t += 1;
            b.begin_pass(w, t);
            b.end_pass(w, 1, t);
        }
        assert!(
            m.evaluate(&b.snapshot(t + 1, 1))
                .iter()
                .all(|s| !matches!(s, Starved::WorkerPoll { .. })),
            "exactly the bound is allowed"
        );
        b.begin_pass(w, t + 2);
        b.end_pass(w, 1, t + 3);
        let f = m.evaluate(&b.snapshot(t + 4, 1));
        assert!(
            f.iter().any(|s| matches!(s, Starved::WorkerPoll { w: 0, passes, .. } if *passes == BOUND_PASSES + 1)),
            "{f:?}"
        );
        // again at once: rate limited
        let again = m.evaluate(&b.snapshot(t + 5, 1));
        assert!(
            again
                .iter()
                .all(|s| !matches!(s, Starved::WorkerPoll { .. }))
        );
        // after a full threshold it may repeat
        let later = m.evaluate(&b.snapshot(t + 5 + thr_ms * MS, 1));
        assert!(
            later
                .iter()
                .any(|s| matches!(s, Starved::WorkerPoll { .. }))
        );
    }

    #[test]
    fn a_worker_stuck_in_one_pass_is_reported_but_a_parked_worker_is_not() {
        let thr = 100;
        let s = snap(500 * MS, |b| b.begin_pass(0, 10));
        let mut m = Monitor::new(thr, &s);
        let f = m.evaluate(&s);
        assert!(
            f.iter()
                .any(|x| matches!(x, Starved::WorkerStuck { w: 0, .. })),
            "{f:?}"
        );
        let parked = snap(500 * MS, |b| {
            b.begin_pass(0, 10);
            b.park(0, true, 11);
        });
        let mut m = Monitor::new(thr, &parked);
        assert!(
            m.evaluate(&parked)
                .iter()
                .all(|x| !matches!(x, Starved::WorkerStuck { .. }))
        );
    }

    // ---- the satisfied-but-unpeeked tracker (deterministic fake clock)

    fn key(target: u64) -> Option<WaitKey> {
        Some(WaitKey {
            kind: WaitKind::Marker,
            target,
        })
    }
    const T: u64 = 1000 * MS;

    #[test]
    fn an_unsatisfied_or_absent_wait_is_never_reported() {
        let mut t = Tracker::new();
        for k in 0..20 {
            assert_eq!(t.observe(k * T, 1, key(3), false, 0, T), None);
            assert_eq!(t.observe(k * T, 2, None, true, 0, T), None);
        }
    }

    #[test]
    fn a_satisfied_wait_nobody_looked_at_is_reported_once_after_the_threshold() {
        let mut t = Tracker::new();
        assert_eq!(
            t.observe(10, 1, key(3), true, 0, T),
            None,
            "first seen true"
        );
        assert_eq!(
            t.observe(10 + T, 1, key(3), true, 0, T),
            None,
            "exactly the threshold"
        );
        assert_eq!(t.observe(11 + T, 1, key(3), true, 0, T), Some(T + 1));
        assert_eq!(
            t.observe(12 + 5 * T, 1, key(3), true, 0, T),
            None,
            "once per condition"
        );
    }

    #[test]
    fn a_look_re_arms_the_condition() {
        let mut t = Tracker::new();
        t.observe(0, 1, key(3), true, 0, T);
        assert!(t.observe(2 * T, 1, key(3), true, 0, T).is_some());
        // the relay is looked at (and stays satisfied: e.g. contended): the interval restarts at the look
        assert_eq!(t.observe(2 * T + 5, 1, key(3), true, 2 * T + 4, T), None);
        assert_eq!(
            t.observe(3 * T + 4, 1, key(3), true, 2 * T + 4, T),
            None,
            "exactly the threshold since the look"
        );
        assert!(
            t.observe(3 * T + 5, 1, key(3), true, 2 * T + 4, T)
                .is_some()
        );
    }

    #[test]
    fn a_look_before_the_wait_was_satisfied_does_not_count_for_the_interval() {
        let mut t = Tracker::new();
        // looked at 100, found unsatisfied; satisfied (first seen) at 500: the interval starts at 500
        assert_eq!(t.observe(500, 1, key(3), true, 100, T), None);
        assert_eq!(t.observe(500 + T, 1, key(3), true, 100, T), None);
        assert!(t.observe(501 + T, 1, key(3), true, 100, T).is_some());
    }

    #[test]
    fn a_changed_wait_or_a_gap_of_unsatisfied_readings_re_arms() {
        let mut t = Tracker::new();
        t.observe(0, 1, key(3), true, 0, T);
        assert!(t.observe(2 * T, 1, key(3), true, 0, T).is_some());
        assert_eq!(
            t.observe(2 * T + 1, 1, key(4), true, 0, T),
            None,
            "a new target is a new condition"
        );
        assert!(t.observe(4 * T, 1, key(4), true, 0, T).is_some());
        assert_eq!(t.observe(4 * T + 1, 1, key(4), false, 0, T), None);
        assert_eq!(
            t.observe(4 * T + 2, 1, key(4), true, 0, T),
            None,
            "the interval restarted"
        );
        assert!(t.observe(6 * T, 1, key(4), true, 0, T).is_some());
    }

    #[test]
    fn relays_are_independent_and_the_set_is_bounded() {
        let mut t = Tracker::new();
        t.observe(0, 1, key(3), true, 0, T);
        t.observe(T, 2, key(3), true, 0, T);
        assert!(t.observe(T + 5, 1, key(3), true, 0, T).is_some());
        assert_eq!(t.observe(T + 5, 2, key(3), true, 0, T), None);
        for id in 10..10 + 2 * MAX_RELAYS as u64 {
            let _ = t.observe(0, id, key(1), true, 0, T);
        }
        assert!(t.len() <= MAX_RELAYS);
        // a relay past the cap is ignored, never a panic, never a report
        assert_eq!(t.observe(100 * T, 10_000, key(1), true, 0, T), None);
    }

    #[test]
    fn retain_forgets_retired_relays() {
        let mut t = Tracker::new();
        t.observe(0, 1, key(3), true, 0, T);
        t.observe(0, 2, key(3), true, 0, T);
        t.retain(&[2]);
        assert_eq!(t.len(), 1);
        assert!(t.open_age_ns(1, 5).is_none());
        assert_eq!(t.open_age_ns(2, 5), Some(5));
    }

    #[test]
    fn the_interval_start_is_the_later_of_first_true_and_last_peek() {
        assert_eq!(interval_start(5, 9), 9);
        assert_eq!(interval_start(9, 5), 9);
        assert_eq!(interval_start(0, 0), 0);
    }

    #[test]
    fn wait_keys_carry_kind_and_target() {
        assert_eq!(
            WaitKey::of(Wait::Marker { target: 7 }),
            WaitKey {
                kind: WaitKind::Marker,
                target: 7
            }
        );
        assert_eq!(
            WaitKey::of(Wait::Popped { target: 9 }),
            WaitKey {
                kind: WaitKind::Popped,
                target: 9
            }
        );
        assert_eq!(
            WaitKey::of(Wait::Contended),
            WaitKey {
                kind: WaitKind::Contended,
                target: 0
            }
        );
        assert_eq!(
            [
                WaitKind::Marker.name(),
                WaitKind::Popped.name(),
                WaitKind::Contended.name()
            ],
            ["marker", "popped", "contended"]
        );
    }

    // ---- the renderer

    /// Every maximal alphanumeric token that starts with a digit is `0x` + hex digits; none is a bare decimal.
    fn assert_hex_only(line: &str) {
        for tok in line.split(|c: char| !c.is_ascii_alphanumeric()) {
            if tok.is_empty() {
                continue;
            }
            if tok.as_bytes()[0].is_ascii_digit() {
                let rest = tok
                    .strip_prefix("0x")
                    .unwrap_or_else(|| panic!("decimal token {tok:?} in {line:?}"));
                assert!(
                    !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_hexdigit()),
                    "{tok:?} in {line:?}"
                );
            }
        }
    }

    fn full_snapshot() -> Snapshot {
        let b = Board::new();
        for _ in 0..MAX_WORKERS {
            let w = b.register().unwrap();
            b.begin_pass(w, 5);
            b.end_pass(w, 1, 6);
            b.poll(w, 7, true);
            b.serving(w, Some(0x3f));
        }
        for d in &TABLE {
            b.hit(d.point, 9);
        }
        b.snapshot(5_000 * MS, 1)
    }

    fn facts() -> RelayFacts {
        RelayFacts {
            key: (0x1c, 0x5),
            host: 0x3f,
            wait: WaitKey {
                kind: WaitKind::Marker,
                target: 0x2a,
            },
            marks: 0x29,
            popped: 0x30,
            unpeeked_ms: 0x3f0,
            last_peek_age_ms: Some(0x3f2),
            threshold_ms: 1000,
            brief: brief(0x40),
            counters: counters(),
        }
    }

    fn brief(run: u32) -> RelayBrief {
        RelayBrief {
            key: (0x1c, 0x5),
            host: 0x3f,
            wait: Some(WaitKey {
                kind: WaitKind::Marker,
                target: 0x2a,
            }),
            contended_run: run,
            retry: true,
            pending: true,
            waiting: true,
            canceled: false,
            stopped: Some(false),
            disabled: None,
            walk_busy: false,
            walk_waiters: Some(2),
        }
    }

    fn counters() -> Vec<(&'static str, u64)> {
        vec![
            ("contended_turns", 0x1234),
            ("wake_backstop", 0x7),
            ("backstop_contended", 0x3),
            ("backstop_found_work", 0),
            ("busy_looks", 0x15ba7),
        ]
    }

    fn threads(n: usize) -> Vec<ThreadCpu> {
        (0..n)
            .map(|i| ThreadCpu {
                tid: 1000 + i as u64,
                comm: "kf3-worker0".into(),
                state: 'R',
                utime: 12345,
                stime: 678,
            })
            .collect()
    }

    #[test]
    fn a_relay_block_names_the_relay_the_wait_the_reads_and_every_point() {
        let lines = render_relay_block(&facts(), &full_snapshot(), &threads(2));
        let all = lines.join("\n");
        assert!(
            lines[0].starts_with("kf3: PROGRESS_STARVED_RELAY "),
            "{}",
            lines[0]
        );
        for needle in [
            "relay=0x1c:0x5",
            "host=0x3f",
            "wait=marker",
            "target=0x2a",
            "marks=0x29",
            "popped=0x30",
            "satisfied_unpeeked_ms=0x3f0",
            "last_peek_age_ms=0x3f2",
            "threshold_ms=0x3e8",
        ] {
            assert!(lines[0].contains(needle), "{needle} not in {}", lines[0]);
        }
        for d in &TABLE {
            assert!(
                all.contains(&format!("{}=", d.name)),
                "point {} missing from the age table",
                d.name
            );
        }
        assert!(all.contains("PROGRESS_WORKER w=0x0 "));
        assert!(all.contains("serving=0x3f"));
        assert!(all.contains("passes_since_poll="));
        assert!(all.contains("PROGRESS_THREAD tid=0x3e8 comm=kf3-worker0 state=R utime_ticks=0x3039 stime_ticks=0x2a6"));
    }

    #[test]
    fn a_never_peeked_relay_and_never_hit_points_read_as_never() {
        let mut f = facts();
        f.last_peek_age_ms = None;
        let b = Board::new();
        let lines = render_relay_block(&f, &b.snapshot(5 * MS, 1), &[]);
        assert!(lines[0].contains("last_peek_age_ms=never"));
        assert!(lines.join("\n").contains("scan=never/0x0"));
    }

    #[test]
    fn every_line_is_hex_only_and_a_block_is_bounded_even_for_the_worst_input() {
        let mut f = facts();
        f.key = (u32::MAX, u32::MAX);
        f.host = u32::MAX;
        f.wait.target = u64::MAX;
        f.marks = u64::MAX;
        f.popped = u64::MAX;
        f.unpeeked_ms = u64::MAX;
        f.last_peek_age_ms = Some(u64::MAX);
        f.threshold_ms = u64::MAX;
        let lines = render_relay_block(&f, &full_snapshot(), &threads(100));
        assert!(lines.len() <= MAX_BLOCK_LINES, "{} lines", lines.len());
        for l in &lines {
            assert!(l.starts_with("kf3: PROGRESS"), "{l}");
            assert!(l.len() <= MAX_LINE_CHARS, "{} chars: {l}", l.len());
            assert!(!l.contains('\n'));
            assert_hex_only(l);
        }
        let ok = render_relay_block(&facts(), &full_snapshot(), &threads(2));
        for l in &ok {
            assert_hex_only(l);
        }
    }

    #[test]
    fn the_starved_line_is_the_documented_format() {
        let s = Starved::Point {
            point: Point::Poll,
            age_ns: 1234 * MS,
            serve_delta: 0x1f4,
        };
        let l = render_starved(&s, 1000);
        assert_eq!(
            l,
            "kf3: PROGRESS_STARVED point=poll age_ms=0x4d2 serve_delta=0x1f4 threshold_ms=0x3e8"
        );
        let w = render_starved(
            &Starved::WorkerPoll {
                w: 1,
                passes: 0x30,
                age_ns: 5 * MS,
            },
            1000,
        );
        assert_eq!(
            w,
            "kf3: PROGRESS_STARVED point=worker_poll age_ms=0x5 w=0x1 passes_since_poll=0x30 bound=0x10"
        );
        let k = render_starved(
            &Starved::WorkerStuck {
                w: 0,
                age_ns: 2000 * MS,
                serving: (1 << 63) | 0x3f,
            },
            1000,
        );
        assert_eq!(
            k,
            "kf3: PROGRESS_STARVED point=worker_stuck age_ms=0x7d0 w=0x0 serving=0x3f"
        );
        let idle = render_starved(
            &Starved::WorkerStuck {
                w: 0,
                age_ns: 2000 * MS,
                serving: 0,
            },
            1000,
        );
        assert!(idle.ends_with("serving=none"), "{idle}");
        for l in [&l, &w, &k, &idle] {
            assert_hex_only(l);
        }
    }

    #[test]
    fn the_printer_caps_blocks_and_summary_lines_then_says_so_once() {
        let mut p = Printer::default();
        for _ in 0..MAX_BLOCKS {
            assert_eq!(p.block(), Admit::Yes);
        }
        assert_eq!(p.block(), Admit::Suppress);
        for _ in 0..100 {
            assert_eq!(p.block(), Admit::No);
        }
        for _ in 0..MAX_STARVED_LINES {
            assert_eq!(p.starved(), Admit::Yes);
        }
        assert_eq!(p.starved(), Admit::Suppress);
        assert_eq!(p.starved(), Admit::No);
        let line = suppression_line("blocks");
        assert!(
            line.contains("PROGRESS_SUPPRESSED") && line.contains("blocks"),
            "{line}"
        );
        assert_hex_only(&line);
    }

    #[test]
    fn a_thread_stat_line_parses_even_with_parentheses_and_spaces_in_the_name() {
        let line = "4242 (kf3 (worker) 0) S 1 4242 4242 0 -1 4194560 100 0 5 0 777 333 0 0 20 0 9 0 12345 1000 200 18446744073709551615";
        let t = parse_thread_stat(4242, line).expect("parses");
        assert_eq!(
            t,
            ThreadCpu {
                tid: 4242,
                comm: "kf3 (worker) 0".into(),
                state: 'S',
                utime: 777,
                stime: 333
            }
        );
        assert!(parse_thread_stat(1, "garbage").is_none());
        assert!(parse_thread_stat(1, "1 (x) S 1 2").is_none(), "too short");
        assert!(parse_thread_stat(1, "1 (x) S 1 1 1 0 -1 0 0 0 0 0 notanumber 3 0 0").is_none());
    }

    // ---- 2026-10-20 (coordinator H-64): contention / demand state in every block and in the summary

    #[test]
    fn the_relay_state_line_prints_the_contention_demand_and_lifecycle_flags() {
        let l = render_relay_state(&brief(0x40));
        assert!(l.starts_with("kf3: PROGRESS_RELAY_STATE "), "{l}");
        for needle in [
            "relay=0x1c:0x5",
            "host=0x3f",
            "wait=marker",
            "contended_run=0x40",
            "retry_budget=0x40",
            "retry_flag=yes",
            "pending=yes",
            "waiting_flag=yes",
            "canceled=no",
            "stopped=no",
            "disabled=unknown",
            "walk_busy=no",
            "walk_waiters=0x2",
        ] {
            assert!(l.contains(needle), "{needle} not in {l}");
        }
        assert_hex_only(&l);
        assert!(l.len() <= MAX_LINE_CHARS);
    }

    /// H-64: the budget is `CONTENTION_RETRIES` lost turns; at or beyond it only the backstop may re-ring the relay.
    #[test]
    fn the_h64_flag_is_set_exactly_when_only_the_backstop_may_re_ring() {
        let budget = crate::user_loop::CONTENTION_RETRIES;
        for (run, want) in [
            (0, "no"),
            (budget - 1, "no"),
            (budget, "yes"),
            (budget + 1, "yes"),
            (u32::MAX, "yes"),
        ] {
            let l = render_relay_state(&brief(run));
            assert!(
                l.contains(&format!("h64_backstop_only={want}")),
                "run {run}: {l}"
            );
        }
        // the flag agrees with the production predicate for every Peek kind
        for run in [0, budget - 1, budget, budget + 1] {
            let spin_may = crate::user_loop::may_retry(run, Peek::Spin);
            let l = render_relay_state(&brief(run));
            assert_eq!(l.contains("h64_backstop_only=yes"), !spin_may, "run {run}");
            assert!(crate::user_loop::may_retry(run, Peek::Backstop));
        }
        // a relay with no demand and no wait is never "stranded"
        let mut idle = brief(budget + 5);
        idle.pending = false;
        idle.retry = false;
        idle.waiting = false;
        idle.wait = None;
        assert!(render_relay_state(&idle).contains("h64_backstop_only=no"));
    }

    #[test]
    fn flags_read_yes_no_or_unknown_never_a_number() {
        let mut b = brief(1);
        b.retry = false;
        b.pending = false;
        b.waiting = false;
        b.canceled = true;
        b.stopped = Some(true);
        b.disabled = Some(false);
        b.walk_busy = true;
        b.walk_waiters = None;
        b.wait = None;
        let l = render_relay_state(&b);
        for needle in [
            "retry_flag=no",
            "pending=no",
            "waiting_flag=no",
            "canceled=yes",
            "stopped=yes",
            "disabled=no",
            "walk_busy=yes",
            "walk_waiters=unknown",
            "wait=none",
        ] {
            assert!(l.contains(needle), "{needle} in {l}");
        }
        assert_hex_only(&l);
    }

    #[test]
    fn counters_are_hex_six_per_line_and_bounded() {
        let lines = render_counters(&counters());
        assert_eq!(lines.len(), 1);
        for needle in [
            "contended_turns=0x1234",
            "wake_backstop=0x7",
            "backstop_contended=0x3",
            "backstop_found_work=0x0",
            "busy_looks=0x15ba7",
        ] {
            assert!(lines[0].contains(needle), "{needle} in {}", lines[0]);
        }
        let many: Vec<(&'static str, u64)> = (0..100).map(|_| ("busy_looks", u64::MAX)).collect();
        let lines = render_counters(&many);
        assert_eq!(lines.len(), MAX_COUNTERS.div_ceil(6));
        assert_eq!(
            lines
                .iter()
                .map(|l| l.matches("busy_looks=").count())
                .sum::<usize>(),
            MAX_COUNTERS
        );
        for l in &lines {
            assert!(l.starts_with("kf3: PROGRESS_COUNTERS "), "{l}");
            assert!(l.len() <= MAX_LINE_CHARS, "{}", l.len());
            assert_hex_only(l);
        }
        assert!(render_counters(&[]).is_empty());
    }

    #[test]
    fn a_relay_block_carries_the_state_the_counters_and_each_workers_parks() {
        let b = Board::new();
        let w = b.register().unwrap();
        b.park(w, true, 100 * MS);
        b.park(w, false, 120 * MS);
        b.park(w, true, 130 * MS);
        b.park(w, false, 140 * MS);
        let w2 = b.register().unwrap();
        let _ = w2;
        let lines = render_relay_block(&facts(), &b.snapshot(1000 * MS, 1), &[]);
        let all = lines.join("\n");
        assert!(all.contains("PROGRESS_RELAY_STATE "));
        assert!(all.contains("contended_run=0x40"));
        assert!(all.contains("PROGRESS_COUNTERS "));
        assert!(all.contains("contended_turns=0x1234"));
        assert!(all.contains("PROGRESS_WORKER w=0x0 "));
        assert!(all.contains("parks=0x2 park_age_ms=0x35c "), "{all}");
        assert!(all.contains("PROGRESS_WORKER w=0x1 "));
        assert!(all.contains("parks=0x0 park_age_ms=never"), "{all}");
        for l in &lines {
            assert_hex_only(l);
        }
    }

    #[test]
    fn the_board_counts_parks_and_stamps_the_last_park() {
        let b = Board::new();
        let w = b.register().unwrap();
        b.park(w, true, 10);
        b.park(w, false, 20);
        b.park(w, true, 30);
        let s = b.snapshot(40, 1);
        assert_eq!((s.workers[0].parks, s.workers[0].park_ns), (2, 30));
        b.park(w, false, 50);
        assert_eq!(b.snapshot(60, 1).workers[0].park_ns, 50);
    }

    #[test]
    fn the_summary_context_lists_the_stranded_relays_first_the_counters_and_every_worker() {
        let budget = crate::user_loop::CONTENTION_RETRIES;
        let mut quiet = brief(0);
        quiet.host = 1;
        quiet.waiting = false;
        quiet.pending = false;
        quiet.retry = false;
        quiet.wait = None;
        let mut stranded = brief(budget);
        stranded.host = 2;
        let mut relays: Vec<RelayBrief> = (0..20).map(|_| quiet.clone()).collect();
        relays.push(stranded);
        let lines = render_summary_context(&full_snapshot(), &counters(), &relays);
        assert!(lines.len() <= MAX_CONTEXT_LINES, "{} lines", lines.len());
        let states: Vec<&String> = lines
            .iter()
            .filter(|l| l.contains("PROGRESS_RELAY_STATE"))
            .collect();
        assert_eq!(states.len(), MAX_SUMMARY_RELAYS);
        assert!(
            states[0].contains("host=0x2") && states[0].contains("h64_backstop_only=yes"),
            "the stranded relay comes first: {}",
            states[0]
        );
        assert!(lines.iter().any(|l| l.contains("PROGRESS_COUNTERS")));
        assert_eq!(
            lines
                .iter()
                .filter(|l| l.contains("PROGRESS_WORKER"))
                .count(),
            MAX_WORKERS
        );
        for l in &lines {
            assert!(l.starts_with("kf3: PROGRESS"), "{l}");
            assert!(l.len() <= MAX_LINE_CHARS);
            assert_hex_only(l);
        }
        assert!(
            render_summary_context(&full_snapshot(), &[], &[])
                .iter()
                .all(|l| l.contains("PROGRESS_WORKER"))
        );
    }

    /// The example in the design doc (section 11) is what the renderer prints.
    #[test]
    fn the_documented_example_block_is_what_the_renderer_prints() {
        let b = Board::new();
        let w0 = b.register().unwrap();
        let w1 = b.register().unwrap();
        for (w, t) in [(w0, 4_900), (w1, 4_950)] {
            b.begin_pass(w, t * MS);
            b.end_pass(w, 2, t * MS + 1);
            b.serving(w, Some(0x3f));
        }
        b.poll(w0, 3_900 * MS, true);
        b.park(w1, true, 4_000 * MS);
        b.park(w1, false, 4_050 * MS);
        b.hit(Point::DispatchOther, 3_100 * MS);
        b.hit(Point::RelayTurn, 4_990 * MS);
        let lines = render_relay_block(&facts(), &b.snapshot(5_000 * MS, 1), &threads(1));
        let want = [
            "kf3: PROGRESS_STARVED_RELAY relay=0x1c:0x5 host=0x3f wait=marker target=0x2a marks=0x29 popped=0x30 satisfied_unpeeked_ms=0x3f0 last_peek_age_ms=0x3f2 threshold_ms=0x3e8",
            "kf3: PROGRESS_RELAY_STATE relay=0x1c:0x5 host=0x3f wait=marker contended_run=0x40 retry_budget=0x40 h64_backstop_only=yes retry_flag=yes pending=yes waiting_flag=yes canceled=no stopped=no disabled=unknown walk_busy=no walk_waiters=0x2",
            "kf3: PROGRESS_COUNTERS contended_turns=0x1234 wake_backstop=0x7 backstop_contended=0x3 backstop_found_work=0x0 busy_looks=0x15ba7",
            "kf3: PROGRESS_POINTS now_ms=0x1388 (name=age_ms/hits) scan=0x31/0x2 serve=0x31/0x2 poll=0x44c/0x1 look=0x44c/0x1",
            "kf3: PROGRESS_POINTS spin_look=never/0x0 park_wake=0x3b6/0x1 dispatch_other=0x76c/0x1 dispatch_completions=never/0x0",
            "kf3: PROGRESS_POINTS doorbell_drain=never/0x0 peek_event=never/0x0 peek_spin=never/0x0 peek_backstop=never/0x0",
            "kf3: PROGRESS_POINTS relay_turn=0xa/0x1 relay_ring=never/0x0",
            "kf3: PROGRESS_WORKER w=0x0 state=running serving=0x3f passes_since_poll=0x0 poll_age_ms=0x44c look_age_ms=0x44c parks=0x0 park_age_ms=never passes=0x1 polls=0x1 looks=0x1",
            "kf3: PROGRESS_WORKER w=0x1 state=running serving=0x3f passes_since_poll=0x1 poll_age_ms=never look_age_ms=never parks=0x1 park_age_ms=0x3b6 passes=0x1 polls=0x0 looks=0x0",
            "kf3: PROGRESS_THREAD tid=0x3e8 comm=kf3-worker0 state=R utime_ticks=0x3039 stime_ticks=0x2a6",
        ];
        assert_eq!(lines, want);
        let starved = render_starved(
            &Starved::Point {
                point: Point::Poll,
                age_ns: 1_100 * MS,
                serve_delta: 0x1f4,
            },
            1000,
        );
        assert_eq!(
            starved,
            "kf3: PROGRESS_STARVED point=poll age_ms=0x44c serve_delta=0x1f4 threshold_ms=0x3e8"
        );
    }

    // ---- the hot path stays cheap and the switch is read once (source pins)

    fn production() -> &'static str {
        include_str!("progress.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap()
    }

    /// The worker-facing functions take no lock, allocate nothing, format nothing, print nothing, sleep nowhere and make no syscall of
    /// their own; every atomic access in them is relaxed.
    #[test]
    fn pin_the_hot_path_has_no_lock_allocation_print_or_syscall() {
        let src = production();
        let hot = [
            "pub fn hit(",
            "pub fn begin_pass(",
            "pub fn end_pass(",
            "pub fn poll(",
            "pub fn park(",
            "pub fn serving(",
            "pub fn look(",
            "pub fn spin(",
            "pub fn park_begin(",
            "pub fn park_end(",
            "pub fn dispatch_other(",
            "pub fn dispatch_completions(",
            "pub fn doorbell_drain(",
            "pub fn peek(",
            "pub fn hit_point(",
            "pub fn set_serving(",
        ];
        for name in hot {
            let at = src.find(name).unwrap_or_else(|| panic!("{name} missing"));
            let body = &src[at..];
            let end = [body.find("\n}\n"), body.find("\n    }\n")]
                .into_iter()
                .flatten()
                .min()
                .unwrap_or(body.len());
            let body = &body[..end];
            for banned in [
                "lock(",
                "Mutex",
                "RwLock",
                "Vec",
                "String",
                "format!",
                "Box",
                "println",
                "sleep",
                "Condvar",
                "File",
                "fs::",
                "libc",
                "syscall",
                "Instant",
                "unsafe",
                "thread::spawn",
                "SeqCst",
                "Acquire",
                "Release",
                "AcqRel",
            ] {
                assert!(!body.contains(banned), "{name} must not contain {banned:?}");
            }
        }
    }

    #[test]
    fn pin_the_clock_is_one_monotonic_read_and_the_environment_is_read_once() {
        let src = production();
        assert_eq!(src.matches("std::env::var").count(), 1);
        assert!(src.contains("OnceLock"));
        assert_eq!(
            src.matches("Instant::now").count(),
            1,
            "the one clock read lives in now_ns"
        );
        assert!(!src.contains("unsafe"));
    }

    #[test]
    fn pin_the_switch_is_off_by_default_and_register_is_none_when_off() {
        // no test sets the variable, so the process-wide setting is Off here
        assert_eq!(threshold_ms(), 0);
        assert!(!on());
        assert!(Tap::register().is_none());
        hit_point(Point::Scan); // nothing happens, no panic
        set_serving(Some(1));
        assert_eq!(BOARD.snapshot(10, 1).points[0], PointSnap::default());
    }
}
