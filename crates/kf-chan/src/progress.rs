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
pub const MAX_BLOCK_LINES: usize = 1 + 3 + MAX_WORKERS + MAX_THREADS;
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
pub fn parse_setting(_value: Option<&str>) -> Setting {
    todo!("EXPECTED RED")
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
pub const TABLE: [Def; POINTS] = [Def {
    point: Point::Scan,
    name: "",
    due: Due::Informational,
    site: "",
}; POINTS];

impl Point {
    /// Position in [`TABLE`].
    #[must_use]
    pub fn index(self) -> usize {
        todo!("EXPECTED RED")
    }
    /// The table row.
    #[must_use]
    pub fn def(self) -> &'static Def {
        &TABLE[self.index()]
    }
    /// The name.
    #[must_use]
    pub fn name(self) -> &'static str {
        self.def().name
    }
}

/// The board's start: [`now_ns`] counts from 1.
pub const START_NS: u64 = 1;

/// The monotonic clock of the board: ns since the first call, never 0.
#[must_use]
pub fn now_ns() -> u64 {
    todo!("EXPECTED RED")
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
    pub fn age(&self, _p: Point) -> Age {
        todo!("EXPECTED RED")
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
                }
            }; MAX_WORKERS],
            next_worker: AtomicUsize::new(0),
        }
    }
    /// Stamp `p` at `now_ns` and count it.
    pub fn hit(&self, _p: Point, _now_ns: u64) {
        todo!("EXPECTED RED")
    }
    /// Register a worker; `None` past [`MAX_WORKERS`].
    pub fn register(&self) -> Option<usize> {
        todo!("EXPECTED RED")
    }
    /// A worker begins a pass.
    pub fn begin_pass(&self, _w: usize, _now_ns: u64) {
        todo!("EXPECTED RED")
    }
    /// A worker's pass returned having served `served` tokens.
    pub fn end_pass(&self, _w: usize, _served: usize, _now_ns: u64) {
        todo!("EXPECTED RED")
    }
    /// A worker polled (`Point::Poll` and the per-worker word); `look` also stamps the busy look.
    pub fn poll(&self, _w: usize, _now_ns: u64, _look: bool) {
        todo!("EXPECTED RED")
    }
    /// A worker entered (`true`) or left the park wait.
    pub fn park(&self, _w: usize, _parked: bool, _now_ns: u64) {
        todo!("EXPECTED RED")
    }
    /// What a worker is serving.
    pub fn serving(&self, _w: usize, _token: Option<u32>) {
        todo!("EXPECTED RED")
    }
    /// Read everything.
    #[must_use]
    pub fn snapshot(&self, _now_ns: u64, _start_ns: u64) -> Snapshot {
        todo!("EXPECTED RED")
    }
}

impl Default for Board {
    fn default() -> Self {
        Self::new()
    }
}

/// The process-wide board (one device per process).
pub static BOARD: Board = Board::new();

/// Read [`ENV`] once; idempotent. Returns the setting in force.
pub fn init_from_env() -> Setting {
    todo!("EXPECTED RED")
}

/// The threshold in ms in force (0 = off): a relaxed load.
#[must_use]
pub fn threshold_ms() -> u64 {
    todo!("EXPECTED RED")
}

/// Whether the diagnostic is on.
#[must_use]
pub fn on() -> bool {
    threshold_ms() != 0
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
        todo!("EXPECTED RED")
    }
    /// The worker's cell index.
    #[must_use]
    pub fn worker(&self) -> usize {
        self.w
    }
    /// See [`Board::begin_pass`].
    pub fn begin_pass(&self) {
        todo!("EXPECTED RED")
    }
    /// See [`Board::end_pass`].
    pub fn end_pass(&self, _served: usize) {
        todo!("EXPECTED RED")
    }
    /// A busy look began.
    pub fn look(&self) {
        todo!("EXPECTED RED")
    }
    /// An idle spin iteration.
    pub fn spin(&self) {
        todo!("EXPECTED RED")
    }
    /// The park wait is about to start.
    pub fn park_begin(&self) {
        todo!("EXPECTED RED")
    }
    /// The park wait ended.
    pub fn park_end(&self) {
        todo!("EXPECTED RED")
    }
    /// A tag was dispatched: engine event.
    pub fn dispatch_other(&self) {
        todo!("EXPECTED RED")
    }
    /// A tag was dispatched: completions.
    pub fn dispatch_completions(&self) {
        todo!("EXPECTED RED")
    }
    /// The doorbell eventfd was drained.
    pub fn doorbell_drain(&self) {
        todo!("EXPECTED RED")
    }
    /// `relay.peek(why)` is about to run.
    pub fn peek(&self, _why: Peek) {
        todo!("EXPECTED RED")
    }
}

/// Hit `p` from the calling worker thread's context (the relay path in `kf-qemu`); nothing when the diagnostic is off.
pub fn hit_point(_p: Point) {
    todo!("EXPECTED RED")
}

/// Record (`Some`) or clear (`None`) the host token the calling worker is serving; nothing when the diagnostic is off or the thread is no worker.
pub fn set_serving(_token: Option<u32>) {
    todo!("EXPECTED RED")
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
    pub fn new(_threshold_ms: u64, _snap: &Snapshot) -> Self {
        todo!("EXPECTED RED")
    }
    /// Evaluate `snap`: point violations once per window, worker violations at most once per threshold each.
    pub fn evaluate(&mut self, _snap: &Snapshot) -> Vec<Starved> {
        todo!("EXPECTED RED")
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
    pub fn name(self) -> &'static str {
        todo!("EXPECTED RED")
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
    pub fn of(_w: Wait) -> Self {
        todo!("EXPECTED RED")
    }
}

/// The start of a satisfied-but-unpeeked interval: the later of the first time it was seen true and the last look.
#[must_use]
pub fn interval_start(_first_true_ns: u64, _last_peek_ns: u64) -> u64 {
    todo!("EXPECTED RED")
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
        _now_ns: u64,
        _id: u64,
        _wait: Option<WaitKey>,
        _satisfied: bool,
        _last_peek_ns: u64,
        _threshold_ns: u64,
    ) -> Option<u64> {
        todo!("EXPECTED RED")
    }
    /// The current age of `id`'s interval, if one is open (for the model comparison).
    #[must_use]
    pub fn open_age_ns(&self, _id: u64, _now_ns: u64) -> Option<u64> {
        todo!("EXPECTED RED")
    }
    /// Forget every relay not in `live`.
    pub fn retain(&mut self, _live: &[u64]) {
        todo!("EXPECTED RED")
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
pub fn parse_thread_stat(_tid: u64, _stat: &str) -> Option<ThreadCpu> {
    todo!("EXPECTED RED")
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
}

/// Render one relay's block: the relay line, the age table of ALL points, every worker, the listed threads. Hex-only, bounded.
#[must_use]
pub fn render_relay_block(
    _f: &RelayFacts,
    _snap: &Snapshot,
    _threads: &[ThreadCpu],
) -> Vec<String> {
    todo!("EXPECTED RED")
}

/// Render one [`Starved`] as its single `PROGRESS_STARVED` line.
#[must_use]
pub fn render_starved(_s: &Starved, _threshold_ms: u64) -> String {
    todo!("EXPECTED RED")
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
        todo!("EXPECTED RED")
    }
    /// Admit a `PROGRESS_STARVED` summary line?
    pub fn starved(&mut self) -> Admit {
        todo!("EXPECTED RED")
    }
}

/// The suppression line.
#[must_use]
pub fn suppression_line(_what: &str) -> String {
    todo!("EXPECTED RED")
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
        }
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
            src.matches("Instant::now()").count(),
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
