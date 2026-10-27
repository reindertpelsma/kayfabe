use super::core::{Cause, Gauge, Kind, NotificationSource, SLOW_NS, Sample, Span, State, Wait};
use super::journal::Journal;
use super::splits::{SplitScope, Splits};
use std::cell::Cell;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{
    AtomicBool, AtomicU64,
    Ordering::{Acquire, Relaxed, Release},
};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const THREADS: usize = 64;
const VECTORS: usize = 64;
// Diagnostics only: at most one fixed-size live dump per second, at most 16,384 per process.
// First-slow and shutdown each retain their independent single dump allowance.
const LIVE_PERIOD_NS: u64 = 1_000_000_000;
const LIVE_DUMPS: u64 = 16_384;
#[derive(Default)]
struct LiveBudget {
    next: AtomicU64,
    issued: AtomicU64,
}
impl LiveBudget {
    fn claim(&self, at: u64) -> bool {
        let issued = self.issued.load(Relaxed);
        let next = self.next.load(Relaxed);
        if issued >= LIVE_DUMPS || at < next || at > u64::MAX - LIVE_PERIOD_NS {
            return false;
        }
        // Single attempts: overlapping polls can lose a diagnostic opportunity, never spin.
        self.next
            .compare_exchange(next, at + LIVE_PERIOD_NS, Relaxed, Relaxed)
            .is_ok()
            && self
                .issued
                .compare_exchange(issued, issued + 1, Relaxed, Relaxed)
                .is_ok()
    }
}
static SESSION: OnceLock<Option<Recorder>> = OnceLock::new();
thread_local! {
    static THREAD: Cell<u64> = const { Cell::new(0) };
    static CONTENTION: Cell<Option<(Cause,u64)>> = const { Cell::new(None) };
}
/// Thread identity in slow-record activity context.
#[derive(Debug, Clone, Copy)]
#[repr(u64)]
pub enum Role {
    /// A channel worker.
    Worker = 1,
    /// The VA-manager thread.
    Va = 2,
    /// The deferred host-act thread.
    Act = 3,
    /// A diagnostic probe thread.
    Probe = 4,
}
/// Fixed, authored activity names; no guest strings or pointers are retained.
#[derive(Debug, Clone, Copy)]
pub enum Activity {
    /// One worker-loop iteration.
    Worker,
    /// A plane worker pass.
    Serve,
    /// Ready-event dispatch.
    Dispatch,
    /// Waiting in epoll (may be normal idle time).
    Park,
    /// A USER pump.
    User,
    /// A Translated pump.
    Translated,
    /// A VA-manager visit.
    Walk,
    /// A deferred act.
    Act,
    /// PT metadata acquisition.
    WaitPt,
    /// PT metadata ownership.
    HeldPt,
    /// Placement row writer ownership.
    HeldRows,
    /// USER registry acquisition.
    UserRegistry,
    /// A failed slot acquisition.
    SlotBusy,
    /// A failed private-storage acquisition.
    StorageBusy,
    /// One authored RM ioctl class (`diag::Verb`).
    Rm(u8),
    /// An authored control's command number.
    Control(u32),
}
impl Activity {
    fn encode(self) -> (u64, u64) {
        match self {
            Self::Worker => (1, 0),
            Self::Serve => (2, 0),
            Self::Dispatch => (3, 0),
            Self::Park => (4, 0),
            Self::User => (5, 0),
            Self::Translated => (6, 0),
            Self::Walk => (7, 0),
            Self::Act => (8, 0),
            Self::WaitPt => (9, 0),
            Self::HeldPt => (10, 0),
            Self::HeldRows => (14, 0),
            Self::UserRegistry => (11, 0),
            Self::SlotBusy => (12, 0),
            Self::StorageBusy => (13, 0),
            Self::Rm(v) => (64, u64::from(v)),
            Self::Control(v) => (65, u64::from(v)),
        }
    }
}
fn label(code: u64, detail: u64) -> String {
    match code {
        1 => "worker iteration".into(),
        2 => "worker serving channels".into(),
        3 => "worker dispatching events".into(),
        4 => "epoll park (idle may be expected)".into(),
        5 => "USER pump".into(),
        6 => "Translated pump".into(),
        7 => "VA loop visit (may include idle)".into(),
        8 => "act thread operation".into(),
        9 => "waiting on PT lock".into(),
        10 => "holding PT lock".into(),
        11 => "waiting on USER registry lock".into(),
        12 => "slot lock contended".into(),
        13 => "private storage lock contended".into(),
        14 => "holding placement rows lock".into(),
        64 => format!(
            "RM ioctl {}",
            [
                "reserve_va",
                "alloc",
                "free",
                "map",
                "unmap",
                "control",
                "invalidate",
                "other"
            ]
            .get(detail as usize)
            .unwrap_or(&"unknown")
        ),
        65 => format!("RM control {detail:#x}"),
        _ => "unknown".into(),
    }
}
#[derive(Default)]
struct Thread {
    role: AtomicU64,
    code: AtomicU64,
    detail: AtomicU64,
    start: AtomicU64,
    last_loop: AtomicU64,
    max_gap: AtomicU64,
}
struct Recorder {
    born: Instant,
    vectors: Vec<u32>,
    state: Mutex<State>,
    notices: Journal,
    operations: Journal,
    threads: [Thread; THREADS],
    next_thread: AtomicU64,
    next_stream: AtomicU64,
    state_lost: AtomicU64,
    notice_seen: AtomicU64,
    notice_gaps: AtomicU64,
    next_due: AtomicU64,
    dumped: AtomicBool,
    shutdown: AtomicBool,
    live: LiveBudget,
}
impl Recorder {
    fn new(born: Instant, vectors: Vec<u32>) -> Self {
        Self {
            born,
            vectors,
            state: Mutex::new(State::default()),
            notices: Journal::default(),
            operations: Journal::default(),
            threads: std::array::from_fn(|_| Thread::default()),
            next_thread: AtomicU64::new(0),
            next_stream: AtomicU64::new(1),
            state_lost: AtomicU64::new(0),
            notice_seen: AtomicU64::new(0),
            notice_gaps: AtomicU64::new(0),
            next_due: AtomicU64::new(u64::MAX),
            dumped: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            live: LiveBudget::default(),
        }
    }
    fn now(&self) -> u64 {
        u64::try_from(self.born.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
    fn spans(&self, at: u64) -> Vec<Span> {
        let mut spans: Vec<_> = self
            .operations
            .read()
            .into_iter()
            .map(|e| Span {
                thread: e.words[0],
                role: e.words[1],
                code: e.words[2],
                detail: e.words[3],
                start: e.words[4],
                end: e.words[5],
            })
            .collect();
        for (i, t) in self.threads.iter().enumerate() {
            let code = t.code.load(Acquire);
            if code != 0 {
                spans.push(Span {
                    thread: i as u64 + 1,
                    role: t.role.load(Relaxed),
                    code,
                    detail: t.detail.load(Relaxed),
                    start: t.start.load(Relaxed),
                    end: at,
                });
            }
        }
        spans
    }
    fn drain(&self, st: &mut State) {
        let mut last = self.notice_seen.load(Relaxed);
        for event in self.notices.read() {
            if event.seq <= last {
                continue;
            }
            self.notice_gaps
                .fetch_add(event.seq.saturating_sub(last + 1), Relaxed);
            let [at, mask, success, source, ..] = event.words;
            if success == 0 {
                st.failed = st.failed.saturating_add(1);
            } else {
                for (i, v) in self.vectors.iter().enumerate() {
                    if mask & (1u64 << i) != 0 {
                        let source = if source == 1 {
                            NotificationSource::Storm
                        } else {
                            NotificationSource::Normal
                        };
                        st.notify_source(*v, at, true, source);
                    }
                }
            }
            last = event.seq;
        }
        self.notice_seen.store(last, Release);
    }
    fn update(&self, f: impl FnOnce(&mut State)) {
        let Ok(mut st) = self.state.try_lock() else {
            self.state_lost.fetch_add(1, Relaxed);
            return;
        };
        f(&mut st);
        self.drain(&mut st);
        let at = self.now();
        st.expire(at);
        if st.slow.iter().any(|s| !s.context_checked)
            || st.splits.slow.iter().any(|s| !s.context_checked)
        {
            let spans = self.spans(at);
            for s in &mut st.slow {
                if s.context_checked {
                    continue;
                }
                let end = s.raised.or(s.advanced).unwrap_or(at).max(s.published);
                let mut work: Vec<_> = spans
                    .iter()
                    .copied()
                    .filter(|w| {
                        w.end >= s.observed
                            && w.start <= end
                            && (w.thread == s.worker
                                || w.role == Role::Va as u64
                                || w.role == Role::Act as u64)
                    })
                    .collect();
                work.sort_by_key(|w| context_key(w, s.observed, end));
                work.truncate(4);
                s.work = work;
                s.context_checked = true;
            }
        }
        if st.splits.slow.iter().any(|s| !s.context_checked) {
            let spans = self.spans(at);
            for s in &mut st.splits.slow {
                if s.context_checked {
                    continue;
                }
                let end = s.taken.or(s.released).unwrap_or(at);
                let mut work: Vec<_> = spans
                    .iter()
                    .copied()
                    .filter(|w| {
                        w.end >= s.requested
                            && w.start <= end
                            && (w.thread == s.worker
                                || w.role == Role::Va as u64
                                || w.role == Role::Act as u64)
                    })
                    .collect();
                work.sort_by_key(|w| context_key(w, s.requested, end));
                work.truncate(4);
                s.work = work;
                s.context_checked = true;
            }
        }
        self.next_due.store(
            st.pending
                .iter()
                .filter(|s| !s.saved)
                .map(|s| s.observed.saturating_add(SLOW_NS + 1))
                .min()
                .unwrap_or(u64::MAX)
                .min(st.splits.next_due()),
            Release,
        );
    }
}
fn recorder() -> Option<&'static Recorder> {
    SESSION.get().and_then(Option::as_ref)
}
/// Initialize before workers start, using exactly the device heartbeat's clock origin.
/// `KF3_DIAG_LATENCY=1` enables it; disabled sessions retain no packet state.
pub fn init(born: Instant, vectors: &[u32]) -> bool {
    SESSION.get_or_init(|| {
        if std::env::var("KF3_DIAG_LATENCY").ok().as_deref() != Some("1") {
            return None;
        }
        let mut list = vectors.to_vec();
        list.sort_unstable();
        list.dedup();
        let overflow = list.len().saturating_sub(VECTORS);
        list.truncate(VECTORS);
        eprintln!(
            "kf3: LATENCY enabled threshold_us=50000 pending_cap=512 slow_keep=64 \
            notification_journal=256 vectors={} vector_overflow={overflow} \
            gpu_timestamp=unavailable correlation=eligible-vector-opportunity",
            list.len()
        );
        Some(Recorder::new(born, list))
    });
    recorder().is_some_and(|r| r.born == born)
}
/// The heartbeat-relative clock, absent on the disabled fast path.
pub fn now() -> Option<u64> {
    recorder().map(Recorder::now)
}
/// Register only known non-vCPU threads; an unregistered caller creates no activity journal.
pub fn register_thread(role: Role) -> u64 {
    let Some(r) = recorder() else {
        return 0;
    };
    THREAD.with(|slot| {
        if slot.get() != 0 {
            return slot.get();
        }
        let id = r.next_thread.fetch_add(1, Relaxed) + 1;
        if let Some(t) = r.threads.get(id as usize - 1) {
            t.role.store(role as u64, Release);
            slot.set(id);
            id
        } else {
            0
        }
    })
}
/// Current registered thread id; zero means unknown/unregistered, never an address.
pub fn thread_id() -> u64 {
    THREAD.with(Cell::get)
}
/// Scope whose completion records an authored operation. No locks or allocation on drop.
#[must_use]
#[derive(Debug)]
pub struct ActivityGuard {
    data: Option<ActiveScope>,
    _local: PhantomData<Rc<()>>,
}
#[derive(Debug, Clone, Copy)]
struct ActiveScope {
    id: u64,
    code: u64,
    detail: u64,
    at: u64,
    previous: (u64, u64, u64),
}
/// Record a worker/VA/act operation. Unregistered callers (including vCPUs) do nothing.
pub fn activity(op: Activity) -> ActivityGuard {
    let data = recorder().and_then(|r| {
        let id = thread_id();
        let t = r.threads.get(id.checked_sub(1)? as usize)?;
        let (code, detail) = op.encode();
        let at = r.now();
        let previous = (
            t.code.load(Relaxed),
            t.detail.load(Relaxed),
            t.start.load(Relaxed),
        );
        t.detail.store(detail, Relaxed);
        t.start.store(at, Relaxed);
        t.code.store(code, Release);
        Some(ActiveScope {
            id,
            code,
            detail,
            at,
            previous,
        })
    });
    ActivityGuard {
        data,
        _local: PhantomData,
    }
}
impl Drop for ActivityGuard {
    fn drop(&mut self) {
        let Some(ActiveScope {
            id,
            code,
            detail,
            at: start,
            previous,
        }) = self.data
        else {
            return;
        };
        let Some(r) = recorder() else {
            return;
        };
        let t = &r.threads[id as usize - 1];
        r.operations
            .push([id, t.role.load(Relaxed), code, detail, start, r.now(), 0]);
        t.detail.store(previous.1, Relaxed);
        t.start.store(previous.2, Relaxed);
        t.code.store(previous.0, Release);
    }
}
/// Record a failed try-lock's known owner (zero if unavailable), with no scheduling effect.
pub fn contention(cause: Cause, owner: u64) {
    if recorder().is_none() {
        return;
    }
    CONTENTION.with(|v| v.set(Some((cause, owner))));
}
/// Most recent failed acquisition in this worker visit; no owner is inferred from another thread.
pub fn last_contention() -> Option<(Cause, u64)> {
    CONTENTION.with(Cell::get)
}
/// New channel visit; prevent a prior channel's contention being attributed to this one.
pub fn visit() {
    if recorder().is_none() {
        return;
    }
    CONTENTION.with(|v| v.set(None));
}
/// One actual worker-loop boundary, plus bounded nonblocking recorder maintenance.
pub fn worker_loop() {
    let Some(r) = recorder() else {
        return;
    };
    let id = thread_id();
    if let Some(t) = id.checked_sub(1).and_then(|i| r.threads.get(i as usize)) {
        let at = r.now();
        let last = t.last_loop.swap(at, Relaxed);
        if last != 0 {
            t.max_gap.fetch_max(at.saturating_sub(last), Relaxed);
        }
    }
    poll();
}
/// Snapshot eligible logical vectors before an IRQ write. Only immutable vectors and atomics.
pub fn notification_mask(mut eligible: impl FnMut(u32) -> bool) -> u64 {
    let Some(r) = recorder() else {
        return 0;
    };
    r.vectors.iter().enumerate().fold(0, |mask, (i, v)| {
        mask | if eligible(*v) { 1u64 << i } else { 0 }
    })
}
/// Actual IRQ write outcome, safe on a vCPU: no mutex, allocation, formatting or packet scan.
/// The physical line is 0; mask names the sampled eligible logical-vector set.
pub fn notification(mask: u64, success: bool) {
    notification_from(mask, success, false);
}
/// Actual diagnostic-storm IRQ outcome, distinguished from normal notification opportunities.
pub fn notification_storm(mask: u64, success: bool) {
    notification_from(mask, success, true);
}
fn notification_from(mask: u64, success: bool, storm: bool) {
    if let Some(r) = recorder() {
        r.notices
            .push([r.now(), mask, u64::from(success), u64::from(storm), 0, 0, 0]);
    }
}
/// Per-native-ring demand context, bounded to eight most recent wait intervals.
#[derive(Debug)]
pub struct Tracker {
    stream: u64,
    token: u32,
    kind: Kind,
    vector: Option<u32>,
    observed: Option<u64>,
    worker: u64,
    wait: Option<(Cause, u64, u64)>,
    waits: Vec<Wait>,
    published: bool,
}
impl Default for Tracker {
    fn default() -> Self {
        Self {
            stream: 0,
            token: 0,
            kind: Kind::Private,
            vector: None,
            observed: None,
            worker: 0,
            wait: None,
            waits: Vec::new(),
            published: false,
        }
    }
}
impl Tracker {
    /// Bind native identity and derived guest vector; a stream id survives retries, not rebirth.
    pub fn configure(&mut self, kind: Kind, token: u32, vector: Option<u32>) {
        if recorder().is_some() {
            self.kind = kind;
            self.token = token;
            self.vector = vector;
        }
    }
    /// First worker observation of a demand/turn; retries preserve the oldest observation.
    pub fn observe(&mut self) {
        let Some(r) = recorder() else {
            return;
        };
        if thread_id() == 0 {
            return;
        }
        if self.stream == 0 {
            self.stream = r.next_stream.fetch_add(1, Relaxed);
        }
        if self.observed.is_none() {
            self.observed = Some(r.now());
            self.worker = thread_id();
        }
    }
    fn close_wait(&mut self, at: u64) {
        if let Some((cause, start, owner)) = self.wait.take() {
            if self.waits.len() == 8 {
                self.waits.remove(0);
            }
            self.waits.push(Wait {
                cause,
                start,
                end: at,
                owner,
            });
        }
    }
    /// Start/change/end an observed gate interval; repeated same-cause probes keep its start.
    pub fn waiting(&mut self, cause: Option<Cause>) {
        let Some(at) = now() else {
            return;
        };
        self.observe();
        if self.stream == 0 {
            return;
        }
        let owner = last_contention()
            .filter(|(last, _)| Some(*last) == cause)
            .map_or(0, |(_, owner)| owner);
        if self.wait.map(|(c, _, o)| (c, o)) != cause.map(|c| (c, owner)) {
            self.close_wait(at);
            self.wait = cause.map(|c| (c, at, owner));
        }
    }
    /// Record only after successful native publication, with the clock sampled at its doorbell.
    pub fn published_at(&mut self, at: Option<u64>, marker: Option<u64>, wrap: bool) {
        let (Some(at), Some(r)) = (at, recorder()) else {
            return;
        };
        self.observe();
        if self.stream == 0 {
            return;
        }
        self.close_wait(at);
        let s = Sample {
            stream: self.stream,
            token: self.token,
            kind: self.kind,
            observed: self.observed.unwrap_or(at),
            published: at,
            marker,
            wrap,
            advanced: None,
            gpu: None,
            vector: self.vector,
            raised: None,
            source: None,
            waits: self.waits.clone(),
            worker: self.worker,
            cookie: 0,
            saved: false,
            work: Vec::new(),
            context_checked: false,
        };
        r.update(|st| {
            st.publish(s);
            if let Some(m) = marker {
                st.cover(self.stream, m, wrap);
            }
        });
        self.published = true;
    }
    /// Preserve the instant the host read the receipt, independent of later parsing/worker work.
    pub fn marker_at(&self, done: u64, at: Option<u64>) {
        let (Some(at), Some(r)) = (at, recorder()) else {
            return;
        };
        if self.stream != 0 {
            r.update(|st| st.marker(self.stream, done, at, None));
        }
    }
    /// Observe a native receipt now. Existing fences carry no GPU timestamp.
    pub fn marker(&self, done: u64) {
        self.marker_at(done, now());
    }
    /// End a publication/idle visit; a held unpublished demand stays until its next visit.
    pub fn end_turn(&mut self, idle: bool) {
        if idle || self.published {
            self.observed = None;
            self.wait = None;
            self.waits.clear();
        }
        self.published = false;
    }
}
impl Drop for Tracker {
    fn drop(&mut self) {
        if self.stream != 0
            && let Some(r) = recorder()
        {
            r.update(|st| st.cancel(self.stream));
        }
    }
}
/// Worker-side maintenance; fast packets emit no log lines.
pub fn poll() {
    let Some(r) = recorder() else {
        return;
    };
    if r.notices.next.load(Acquire) != r.notice_seen.load(Acquire)
        || r.now() >= r.next_due.load(Acquire)
    {
        r.update(|_| {});
    }
    let first = r.state.try_lock().is_ok_and(|st| st.first_dump);
    if first && !r.dumped.swap(true, Relaxed) {
        dump("first-slow");
    } else if first && r.live.claim(r.now()) {
        // The harness may kill QEMU without running shutdown. Export while it is alive.
        dump("live");
    }
}
fn gauge(g: Gauge) -> String {
    format!("{}/{}", g.count, g.max / 1000)
}
/// Per-stage counts/maxima, missing coverage and a heartbeat/UTC clock pair.
pub fn summary() -> String {
    let Some(r) = recorder() else {
        return "latency[off]".into();
    };
    r.update(|_| {});
    let Ok(st) = r.state.try_lock() else {
        return "latency[snapshot-contended]".into();
    };
    let split = format!(
        "split[count/max_us] queue_walk={} walk_ready={} ready_release={} \
        release_resume={} total={} pending={} slow={} overflow={} missing={} canceled={}",
        gauge(st.splits.gauges[0]),
        gauge(st.splits.gauges[1]),
        gauge(st.splits.gauges[2]),
        gauge(st.splits.gauges[3]),
        gauge(st.splits.gauges[4]),
        st.splits.pending.len(),
        st.splits.slow.len(),
        st.splits.overflow,
        st.splits.missing,
        st.splits.canceled
    );
    let at = r.now();
    let utc = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |v| v.as_nanos());
    format!(
        "LATENCY host_ns={at} unix_ns={utc} basis=host-observed-unless-gpu-present \
        stage[count/max_us] observe_publish={} publish_marker={} marker_raise={} \
        observation_lag_gpu_known={} end_to_end={} gpu_time_missing={} pending={} \
        pending_overflow={} canceled={} early_raise={} invalid={} failed_signal={} \
        private_no_guest_vector={} metadata_lost={} irq_journal_lost={} irq_journal_gaps={} \
        activity_journal_lost={} slow_kept={} live_dumps={}/{} {split}",
        gauge(st.gauges[0]),
        gauge(st.gauges[1]),
        gauge(st.gauges[2]),
        gauge(st.gauges[3]),
        gauge(st.total),
        st.marker_unknown,
        st.pending.len(),
        st.overflow,
        st.canceled,
        st.early,
        st.invalid,
        st.failed,
        st.private_done,
        r.state_lost.load(Relaxed),
        r.notices.lost.load(Relaxed),
        r.notice_gaps.load(Relaxed),
        r.operations.lost.load(Relaxed),
        st.slow.len(),
        r.live.issued.load(Relaxed),
        LIVE_DUMPS
    )
}
fn dump(why: &str) {
    let Some(r) = recorder() else {
        return;
    };
    eprintln!("kf3: LATENCY DUMP why={why} {}", summary());
    let samples = r
        .state
        .try_lock()
        .map(|st| st.slow.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    for s in samples {
        let context = s
            .work
            .iter()
            .map(|w| {
                format!(
                    "thread={} role={} {} detail={} \
            [{},{}]",
                    w.thread,
                    w.role,
                    label(w.code, w.detail),
                    w.detail,
                    w.start,
                    w.end
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        let total = s.raised.and_then(|at| at.checked_sub(s.observed));
        let waits = s
            .waits
            .iter()
            .map(|w| format!("{:?}:{}..{}:owner={}", w.cause, w.start, w.end, w.owner))
            .collect::<Vec<_>>()
            .join(",");
        eprintln!(
            "kf3: LATENCY SLOW id={} stream={} kind={:?} token={:#x} worker={} \
            observe_ns={} publish_ns={} marker_id={:?} marker_observed_ns={:?} \
            marker_gpu_ns={:?} vector={:?} notification_ns={:?} \
            notification_source={:?} irq_line=0 \
            correlation=eligible-vector-opportunity stages_ns={:?} total_ns={total:?} \
            gpu_observation_lag_ns={:?} waits=[{waits}] worker_context=[{context}]",
            s.cookie,
            s.stream,
            s.kind,
            s.token,
            s.worker,
            s.observed,
            s.published,
            s.marker,
            s.advanced,
            s.gpu,
            s.vector,
            s.raised,
            s.source,
            s.stages(),
            s.observation_lag()
        );
    }
    let splits = r
        .state
        .try_lock()
        .map(|st| st.splits.slow.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    for s in splits {
        let context = s
            .work
            .iter()
            .map(|w| {
                format!(
                    "thread={} role={} {} [{},{}]",
                    w.thread,
                    w.role,
                    label(w.code, w.detail),
                    w.start,
                    w.end
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        eprintln!(
            "kf3: LATENCY SPLIT ticket={} token={:#x} scope={:?} \
            requested_ns={} queued_ns={:?} walk_start_ns={:?} apply_end_ns={:?} \
            ready_ns={:?} released_ns={:?} resumed_ns={:?} success={:?} walk={} walks={} \
            last_space={:#x} targets={} walk_wall_ns={} rm_calls_by_verb={:?} \
            diff_runs={} mapped={} unmapped={} shared_walk_cost=true worker={} context=[{context}]",
            s.ticket,
            s.token,
            s.scope,
            s.requested,
            s.queued,
            s.started,
            s.applied,
            s.ready,
            s.released,
            s.taken,
            s.success,
            s.walk,
            s.walks,
            s.space,
            s.targets,
            s.wall_ns,
            s.rm,
            s.diff,
            s.mapped,
            s.unmapped,
            s.worker
        );
    }
    for (i, t) in r
        .threads
        .iter()
        .enumerate()
        .filter(|(_, t)| t.role.load(Relaxed) != 0)
    {
        eprintln!(
            "kf3: LATENCY THREAD id={} role={} loop_gap_max_us={} active={} \
            active_since_ns={}",
            i + 1,
            t.role.load(Relaxed),
            t.max_gap.load(Relaxed) / 1000,
            label(t.code.load(Acquire), t.detail.load(Relaxed)),
            t.start.load(Relaxed)
        );
    }
}
/// One bounded shutdown dump, from a non-vCPU thread after work has stopped.
pub fn dump_shutdown() {
    if let Some(r) = recorder()
        && !r.shutdown.swap(true, Relaxed)
    {
        r.update(|_| {});
        dump("shutdown");
    }
}

fn split_update(f: impl FnOnce(&mut Splits, u64)) {
    if thread_id() == 0 {
        return;
    }
    if let Some(r) = recorder() {
        let at = r.now();
        r.update(|st| f(&mut st.splits, at));
    }
}
/// Worker reached a split; retain the pre-enqueue clock only after enqueue succeeds.
pub fn split_request_at(ticket: u64, token: u32, scope: SplitScope, requested: Option<u64>) {
    if let Some(at) = requested {
        split_update(|s, _| s.request(ticket, token, scope, thread_id(), at));
    }
}
/// Split entered the VA request queue.
pub fn split_queued(ticket: u64) {
    split_update(|s, at| s.queued(ticket, at));
}
/// A successful walker submission, using the clock sampled immediately before submission.
pub fn split_start_at(ticket: u64, walk: u64, space: u64, targets: usize, started: Option<u64>) {
    if let Some(at) = started {
        split_update(|s, _| s.start(ticket, walk, space, targets, at));
    }
}
/// Actual changed-run size and calling-thread RM counters for one applied target.
pub fn split_applied(ticket: u64, space: u64, diff: u64, rm: [u64; 8], maps: [u64; 2]) {
    split_update(|s, at| s.applied(ticket, space, diff, rm, maps, at));
}
/// Complete verdict exists, before starting another queued walk.
pub fn split_ready(ticket: u64, success: bool) {
    split_update(|s, at| s.ready(ticket, success, at));
}
/// Gate release (if any) and result publication have completed.
pub fn split_released(ticket: u64, success: bool) {
    split_update(|s, at| s.released(ticket, success, at));
}
/// Worker actually consumed the result, distinct from a VA-thread signal.
pub fn split_taken(ticket: u64) {
    split_update(|s, at| s.taken(ticket, at));
}
/// Reset/retirement abandoned an old ticket; no completion is inferred.
pub fn split_canceled(ticket: u64) {
    split_update(|s, _| s.cancel(ticket));
}

fn context_key(w: &Span, start: u64, end: u64) -> std::cmp::Reverse<(u8, u64)> {
    let priority = match w.code {
        64.. => 3,
        8..=10 | 14 => 2,
        _ => 0,
    };
    std::cmp::Reverse((priority, w.end.min(end).saturating_sub(w.start.max(start))))
}
#[cfg(test)]
mod context_tests {
    use super::*;
    #[test]
    fn ens11_rm_operation_is_not_hidden_by_its_longer_parent_iteration() {
        let parent = Span {
            code: 1,
            start: 0,
            end: 100,
            ..Span::default()
        };
        let rm = Span {
            code: 64,
            start: 20,
            end: 80,
            ..Span::default()
        };
        assert!(context_key(&rm, 0, 100) < context_key(&parent, 0, 100));
    }
    #[test]
    fn ens11_named_lock_owner_is_not_hidden_by_the_va_loop_scope() {
        let parent = Span {
            code: 7,
            start: 0,
            end: 100,
            ..Span::default()
        };
        let held = Span {
            code: 14,
            start: 25,
            end: 75,
            ..Span::default()
        };
        assert!(context_key(&held, 0, 100) < context_key(&parent, 0, 100));
    }
}

#[cfg(test)]
mod native_recorder_tests {
    use super::*;
    fn native_tracker_pipeline(name: &str, kind: Kind, wrap: bool) {
        if std::env::var("KF_ENS11_NATIVE_CHILD").ok().as_deref() != Some(name) {
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", name, "--nocapture", "--test-threads=1"])
                .env("KF_ENS11_NATIVE_CHILD", name)
                .env("KF3_DIAG_LATENCY", "1")
                .output()
                .unwrap();
            assert!(
                child.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&child.stdout),
                String::from_utf8_lossy(&child.stderr)
            );
            return;
        }
        // Isolated process: exercise real public hooks with the opt-in enabled, no GPU or RM.
        assert!(init(Instant::now(), &[19]));
        assert_ne!(register_thread(Role::Worker), 0);
        let mut tracker = Tracker::default();
        tracker.configure(kind, 0x80c, Some(19));
        tracker.observe();
        contention(Cause::PtLock, 999);
        tracker.waiting(Some(Cause::OwnWalk));
        tracker.waiting(Some(Cause::PtLock));
        let rm = activity(Activity::Rm(5));
        let control = activity(Activity::Control(0x1234));
        split_request_at(1, 0x80c, SplitScope::Space(0x2a3), now());
        split_queued(1);
        std::thread::sleep(std::time::Duration::from_millis(55));
        drop(control);
        drop(rm);
        tracker.published_at(now(), None, wrap);
        tracker.published_at(now(), Some(7), wrap);
        tracker.marker_at(6, now());
        let r = recorder().unwrap();
        {
            let st = r.state.lock().unwrap();
            assert_eq!(st.pending.len(), 2);
            assert!(
                st.pending
                    .iter()
                    .all(|s| s.marker == Some(7) && s.advanced.is_none())
            );
        }
        tracker.marker(7);
        let mask = notification_mask(|vector| vector == 19);
        assert_eq!(mask, 1);
        notification(mask, false);
        poll();
        assert_eq!(r.state.lock().unwrap().pending.len(), 2);
        let expected = if kind == Kind::Translated {
            notification_storm(mask, true);
            NotificationSource::Storm
        } else {
            notification(mask, true);
            NotificationSource::Normal
        };
        split_start_at(1, 3, 0x2a3, 1, now());
        split_applied(1, 0x2a3, 3, [1; 8], [2, 1]);
        split_ready(1, true);
        split_released(1, true);
        split_taken(1);
        split_request_at(999, 0x80c, SplitScope::Pdb(None), now());
        split_queued(999);
        split_canceled(999);
        poll();
        let st = r.state.lock().unwrap();
        assert!(st.pending.is_empty());
        assert_eq!(st.total.count, 2);
        assert_eq!(st.slow.len(), 2);
        for sample in &st.slow {
            assert_eq!(sample.kind, kind);
            assert_eq!(sample.token, 0x80c);
            assert_eq!(sample.source, Some(expected));
            assert_eq!(sample.gpu, None);
            assert!(sample.stages().unwrap()[0] >= SLOW_NS);
            assert!(
                sample
                    .waits
                    .iter()
                    .any(|w| w.cause == Cause::OwnWalk && w.owner == 0)
            );
            assert!(
                sample
                    .waits
                    .iter()
                    .any(|w| w.cause == Cause::PtLock && w.owner == 999)
            );
            assert!(sample.work.iter().any(|w| w.code == 64));
            assert!(
                sample
                    .work
                    .iter()
                    .any(|w| w.code == 65 && w.detail == 0x1234)
            );
        }
        let split = &st.splits.slow[0];
        assert!(split.queued.is_some() && split.started.is_some() && split.applied.is_some());
        assert!(split.ready.is_some() && split.released.is_some() && split.taken.is_some());
        assert_eq!(split.rm, [1; 8]);
        assert_eq!(split.diff, 3);
        assert_eq!([split.mapped, split.unmapped], [2, 1]);
        assert_eq!(st.splits.gauges[4].count, 1);
        assert_eq!(st.splits.canceled, 1);
        assert!(st.splits.pending.is_empty());
        assert_eq!(r.state_lost.load(Relaxed), 0);
    }
    #[test]
    fn ens11_native_user_tracker_records_real_hook_calls_in_an_isolated_process() {
        native_tracker_pipeline(
            concat!(
                "latency::runtime::native_recorder_tests::",
                "ens11_native_user_tracker_records_real_hook_calls_in_an_isolated_process"
            ),
            Kind::User,
            false,
        );
    }
    #[test]
    fn ens11_native_translated_tracker_records_real_hook_calls_in_an_isolated_process() {
        native_tracker_pipeline(
            concat!(
                "latency::runtime::native_recorder_tests::",
                "ens11_native_translated_tracker_records_real_hook_calls_in_an_isolated_process"
            ),
            Kind::Translated,
            true,
        );
    }
    fn publication() -> Sample {
        Sample {
            stream: 1,
            token: 0x80c,
            kind: Kind::Translated,
            observed: 10,
            published: 20,
            marker: Some(1),
            wrap: false,
            advanced: None,
            gpu: None,
            vector: Some(19),
            raised: None,
            source: None,
            waits: Vec::new(),
            worker: 1,
            cookie: 0,
            saved: false,
            work: Vec::new(),
            context_checked: false,
        }
    }
    fn clocked_recorder() -> Recorder {
        Recorder::new(
            Instant::now() - std::time::Duration::from_millis(200),
            vec![19],
        )
    }
    #[test]
    fn ens11_native_recorder_flood_keeps_fixed_packet_and_history_caps() {
        let r = clocked_recorder();
        r.update(|st| {
            for stream in 0..544 {
                let mut s = publication();
                s.stream = stream;
                st.publish(s);
            }
        });
        let st = r.state.lock().unwrap();
        assert_eq!(st.pending.len(), 512);
        assert_eq!(st.slow.len(), 64);
        assert_eq!(st.overflow, 32);
    }
    #[test]
    fn ens11_native_atomic_journal_preserves_normal_and_storm_sources() {
        for (flag, expected) in [
            (0, NotificationSource::Normal),
            (1, NotificationSource::Storm),
        ] {
            let r = clocked_recorder();
            r.notices.push([100_000_000, 1, 1, flag, 0, 0, 0]);
            r.update(|st| {
                st.publish(publication());
                st.marker(1, 1, 30, None);
            });
            let st = r.state.lock().unwrap();
            assert_eq!(st.slow[0].source, Some(expected));
            assert_eq!(st.slow[0].gpu, None);
            assert_eq!(st.gauges[3].count, 0);
        }
    }
    #[test]
    fn ens11_native_receipt_cannot_complete_an_old_stream_with_the_same_guest_token() {
        let r = clocked_recorder();
        r.notices.push([100_000_000, 1, 1, 0, 0, 0, 0]);
        r.update(|st| {
            st.publish(publication());
            let mut reborn = publication();
            reborn.stream = 2;
            st.publish(reborn);
            st.marker(2, 1, 30, None);
        });
        let st = r.state.lock().unwrap();
        assert_eq!(st.pending.len(), 1);
        assert_eq!(st.pending[0].stream, 1);
        assert_eq!(st.pending[0].advanced, None);
        assert_eq!(st.total.count, 1);
    }
    #[test]
    fn ens11_native_vector_mask_mapping_cannot_complete_a_different_logical_vector() {
        let r = Recorder::new(
            Instant::now() - std::time::Duration::from_millis(200),
            vec![20],
        );
        r.notices.push([100_000_000, 1, 1, 0, 0, 0, 0]);
        r.update(|st| {
            st.publish(publication());
            st.marker(1, 1, 30, None);
        });
        let st = r.state.lock().unwrap();
        assert_eq!(st.pending.len(), 1);
        assert_eq!(st.slow[0].vector, Some(19));
        assert_eq!(st.slow[0].raised, None);
        assert_eq!(st.total.count, 0);
    }
    #[test]
    fn ens11_native_failed_or_masked_irq_is_not_a_packet_notification() {
        for (mask, success) in [(0, 1), (1, 0)] {
            let r = clocked_recorder();
            r.notices.push([100_000_000, mask, success, 0, 0, 0, 0]);
            r.update(|st| {
                st.publish(publication());
                st.marker(1, 1, 30, None);
            });
            let st = r.state.lock().unwrap();
            assert_eq!(st.pending.len(), 1);
            assert_eq!(st.slow[0].raised, None);
            assert_eq!(st.total.count, 0);
        }
    }
    #[test]
    fn ens11_native_recorder_names_the_row_lock_held_during_a_slow_interval() {
        let r = clocked_recorder();
        r.operations
            .push([2, Role::Va as u64, 7, 0, 0, 200_000_000, 0]);
        r.operations
            .push([2, Role::Va as u64, 14, 0, 1, 190_000_000, 0]);
        r.update(|st| st.publish(publication()));
        let st = r.state.lock().unwrap();
        assert_eq!(st.slow[0].work[0].code, 14);
        assert!(label(14, 0).contains("placement rows"));
    }
    #[test]
    fn ens11_native_recorder_drops_contended_metadata_without_blocking_the_caller() {
        let r = clocked_recorder();
        let held = r.state.lock().unwrap();
        r.update(|_| panic!("contended metadata must be dropped"));
        assert_eq!(r.state_lost.load(Relaxed), 1);
        drop(held);
        r.update(|_| {});
        assert_eq!(r.state_lost.load(Relaxed), 1);
    }
}

#[cfg(test)]
mod live_export_tests {
    use super::*;

    #[test]
    fn ens12_live_poll_replays_do_not_spend_another_dump() {
        let b = LiveBudget::default();
        assert!(b.claim(100));
        for _ in 0..10_000 {
            assert!(!b.claim(100));
        }
        assert_eq!(b.issued.load(Relaxed), 1);
    }

    #[test]
    fn ens12_live_poll_waits_a_full_period_and_does_not_catch_up() {
        let b = LiveBudget::default();
        assert!(b.claim(100));
        assert!(!b.claim(LIVE_PERIOD_NS + 99));
        assert!(b.claim(100 * LIVE_PERIOD_NS));
        assert!(!b.claim(100 * LIVE_PERIOD_NS));
        assert!(!b.claim(u64::MAX));
    }

    #[test]
    fn ens12_live_session_is_terminal_after_its_last_credit() {
        let b = LiveBudget::default();
        b.issued.store(LIVE_DUMPS - 1, Relaxed);
        assert!(b.claim(0));
        assert!(!b.claim(2 * LIVE_PERIOD_NS));
        assert_eq!(b.issued.load(Relaxed), LIVE_DUMPS);
    }

    #[test]
    fn ens12_live_hostile_increasing_clock_cannot_extend_the_session_budget() {
        let b = LiveBudget::default();
        for tick in 0..(LIVE_DUMPS + 100) {
            assert_eq!(b.claim(tick * LIVE_PERIOD_NS), tick < LIVE_DUMPS);
        }
        assert_eq!(b.issued.load(Relaxed), LIVE_DUMPS);
    }

    #[test]
    fn ens12_concurrent_probes_share_one_dump_credit_per_period() {
        let b = LiveBudget::default();
        let grants = AtomicU64::new(0);
        std::thread::scope(|scope| {
            for _ in 0..32 {
                scope.spawn(|| {
                    if b.claim(0) {
                        grants.fetch_add(1, Relaxed);
                    }
                });
            }
        });
        assert_eq!(grants.load(Relaxed), 1);
        assert_eq!(b.issued.load(Relaxed), 1);
    }

    fn live_case(name: &str, pending_first: bool) {
        let name = name.split_once("::").unwrap().1;
        if std::env::var("KF_ENS12_CHILD").ok().as_deref() != Some(name) {
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", name, "--nocapture", "--test-threads=1"])
                .env("KF_ENS12_CHILD", name)
                .env("KF3_DIAG_LATENCY", "1")
                .output()
                .unwrap();
            let stderr = String::from_utf8_lossy(&child.stderr);
            assert!(child.status.success(), "{stderr}");
            assert!(stderr.contains("why=first-slow"), "{stderr}");
            assert!(!stderr.contains("why=shutdown"), "{stderr}");
            let live = stderr.split("why=live").skip(1).collect::<Vec<_>>();
            assert!(!live.is_empty(), "late split was never exported: {stderr}");
            assert!(
                live.iter().any(|text| text.lines().any(|line| {
                    line.contains("LATENCY SPLIT ticket=7402 ")
                        && line.contains("released_ns=Some(")
                        && line.contains("resumed_ns=Some(")
                })),
                "final split stages were never exported: {stderr}"
            );
            return;
        }
        assert!(init(
            Instant::now() - std::time::Duration::from_secs(10),
            &[19]
        ));
        register_thread(Role::Probe);
        let r = recorder().unwrap();
        r.update(|st| st.splits.request(1, 0x80c, SplitScope::Pdb(None), 1, 1));
        poll(); // Consume the real first-slow dump with boot-time history.
        r.update(|st| {
            st.splits.request(7402, 0x80c, SplitScope::Space(9), 1, 2);
            st.splits.queued(7402, 3);
            st.splits.start(7402, 4, 9, 1, 4);
        });
        if pending_first {
            std::thread::sleep(std::time::Duration::from_millis(1010));
            poll(); // An in-flight export must not consume the later terminal stages.
        }
        r.update(|st| {
            st.splits.applied(7402, 9, 1, [1; 8], [1, 0], SLOW_NS + 5);
            st.splits.ready(7402, true, SLOW_NS + 6);
            st.splits.released(7402, true, SLOW_NS + 7);
            st.splits.taken(7402, SLOW_NS + 8);
        });
        std::thread::sleep(std::time::Duration::from_millis(1010));
        poll(); // No shutdown callback: the next harness action could be SIGKILL.
    }

    #[test]
    fn ens12_late_completed_split_reaches_stderr_without_shutdown() {
        live_case(
            concat!(
                module_path!(),
                "::",
                "ens12_late_completed_split_reaches_stderr_without_shutdown"
            ),
            false,
        );
    }

    #[test]
    fn ens12_live_pending_export_does_not_hide_later_release_and_resume() {
        live_case(
            concat!(
                module_path!(),
                "::",
                "ens12_live_pending_export_does_not_hide_later_release_and_resume"
            ),
            true,
        );
    }
}
