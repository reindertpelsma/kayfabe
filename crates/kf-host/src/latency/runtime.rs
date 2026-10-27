use super::core::{Cause, Gauge, Kind, SLOW_NS, Sample, Span, State, Wait};
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
        7 => "VA walk/apply".into(),
        8 => "act thread operation".into(),
        9 => "waiting on PT lock".into(),
        10 => "holding PT lock".into(),
        11 => "waiting on USER registry lock".into(),
        12 => "slot lock contended".into(),
        13 => "private storage lock contended".into(),
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
            let [at, mask, success, ..] = event.words;
            if success == 0 {
                st.failed = st.failed.saturating_add(1);
            } else {
                for (i, v) in self.vectors.iter().enumerate() {
                    if mask & (1u64 << i) != 0 {
                        st.notify(*v, at, true);
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
                work.sort_by_key(|w| {
                    std::cmp::Reverse(w.end.min(end).saturating_sub(w.start.max(s.observed)))
                });
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
                work.sort_by_key(|w| {
                    std::cmp::Reverse(w.end.min(end).saturating_sub(w.start.max(s.requested)))
                });
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
pub struct ActivityGuard {
    data: Option<(u64, u64, u64, u64, (u64, u64, u64))>,
    _local: PhantomData<Rc<()>>,
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
        Some((id, code, detail, at, previous))
    });
    ActivityGuard {
        data,
        _local: PhantomData,
    }
}
impl Drop for ActivityGuard {
    fn drop(&mut self) {
        let Some((id, code, detail, start, previous)) = self.data else {
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
    if let Some(r) = recorder() {
        r.notices
            .push([r.now(), mask, u64::from(success), 0, 0, 0, 0]);
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
        let owner = last_contention().map_or(0, |(_, owner)| owner);
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
        if self.stream != 0 {
            if let Some(r) = recorder() {
                r.update(|st| st.cancel(self.stream));
            }
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
        activity_journal_lost={} slow_kept={} {split}",
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
        st.slow.len()
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
            marker_gpu_ns={:?} vector={:?} notification_ns={:?} irq_line=0 \
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
    if let Some(r) = recorder() {
        if !r.shutdown.swap(true, Relaxed) {
            r.update(|_| {});
            dump("shutdown");
        }
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
