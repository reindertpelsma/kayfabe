//! ★★★ **Stall instruments** — the always-on, lock-free measurements that answer *"did a thread that
//! serves new input ever stall, and for how long?"* (`docs/design/V3_NONSTALL_THREADS.md`).
//!
//! Owner rule (2026-10-09, binding): the register drainer and every thread that serves new input
//! (the act thread `kf3-chan-act`, the workers, the doorbell servicer) may NEVER stall. A STALL is
//! any state in which the thread cannot serve another input: a sleep, a timed wait, an
//! acknowledgement wait, a contended blocking lock, a blocking write, an unbounded loop. A wait
//! inside an epoll that also accepts new requests is not a stall.
//!
//! ## What is measured, and how it costs
//!
//! Every instrument is a handful of relaxed atomics; none takes a lock, allocates or prints, and
//! none is behind a flag (a Windows boot carries them with no environment variable). The only
//! clock reads are one [`Instant::now`] pair per drainer iteration / per `apply_register` / per
//! act step, and one per CONTENDED lock acquisition (the uncontended path is `try_lock`, exactly
//! as cheap as `lock`).
//!
//! | name in the status line | what | falsifies |
//! |---|---|---|
//! | `drain_pass_max_us` | longest drainer iteration, loop top to park (or to the next iteration) | "the drainer is always back at its poll within N µs" |
//! | `apply_max_us` | longest single `apply_register` | "no privileged write takes the drainer for more than N µs" |
//! | `held_age_max_us` / `held_oldest_us` | longest a guest reply was held, and the age of the oldest reply held now | "held replies are released promptly" |
//! | `act_wait_max_us` | longest an act sat in the queue before its first step ran | "an RPC whose reply is a host act does not wait behind another act" |
//! | `act_run_max_us` | longest single synchronous act step (the time the act thread cannot serve anything) | "no act step blocks the act thread for more than N µs" |
//! | `lock_wait_max_us`, `drainer_lock_wait_max_us`, `act_lock_wait_max_us` | longest time any thread (resp. the drainer, the act thread) BLOCKED on a [`TimedMutex`] / [`TimedRwLock`], with the lock's name | "the drainer never waits on a lock another thread holds" |
//! | `log_dropped` | log lines dropped because the logger's queue was full | "logging never blocked a serving thread" (a drop is the price of not blocking) |
//!
//! Each gauge also counts the samples that reached [`STALL_NS`] (`over10ms`): a count, so a single
//! early outlier in a long boot does not hide a later change.

use std::cell::Cell;
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{
    LockResult, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard, TryLockResult,
};
use std::time::{Duration, Instant};

/// A sample at or over this many nanoseconds (10 ms) is counted as a stall by [`MaxGauge::over`].
pub const STALL_NS: u64 = 10_000_000;

/// A count, a sum, a maximum and the number of samples at or over [`STALL_NS`] — relaxed atomics.
#[derive(Debug, Default)]
pub struct MaxGauge {
    n: AtomicU64,
    sum_ns: AtomicU64,
    max_ns: AtomicU64,
    over: AtomicU64,
}

impl MaxGauge {
    /// An empty gauge.
    #[must_use]
    pub const fn new() -> Self {
        MaxGauge {
            n: AtomicU64::new(0),
            sum_ns: AtomicU64::new(0),
            max_ns: AtomicU64::new(0),
            over: AtomicU64::new(0),
        }
    }

    /// Record one sample of `ns` nanoseconds.
    pub fn record_ns(&self, ns: u64) {
        self.n.fetch_add(1, Ordering::Relaxed);
        self.sum_ns.fetch_add(ns, Ordering::Relaxed);
        self.max_ns.fetch_max(ns, Ordering::Relaxed);
        if ns >= STALL_NS {
            self.over.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record one sample of duration `d` (saturating at `u64::MAX` ns).
    pub fn record(&self, d: Duration) {
        self.record_ns(u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
    }

    /// Record the time since `t0`.
    pub fn since(&self, t0: Instant) {
        self.record(t0.elapsed());
    }

    /// Samples recorded.
    #[must_use]
    pub fn count(&self) -> u64 {
        self.n.load(Ordering::Relaxed)
    }

    /// The largest sample, ns.
    #[must_use]
    pub fn max_ns(&self) -> u64 {
        self.max_ns.load(Ordering::Relaxed)
    }

    /// The largest sample, µs.
    #[must_use]
    pub fn max_us(&self) -> u64 {
        self.max_ns() / 1000
    }

    /// Samples at or over [`STALL_NS`].
    #[must_use]
    pub fn over(&self) -> u64 {
        self.over.load(Ordering::Relaxed)
    }

    /// The sum of every sample, ns.
    #[must_use]
    pub fn sum_ns(&self) -> u64 {
        self.sum_ns.load(Ordering::Relaxed)
    }
}

/// What a thread does, for attributing a lock wait to the drainer or the act thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Anything not named below (workers, the VA thread, the main loop …).
    Other,
    /// The register drainer.
    Drainer,
    /// The channel plane's act thread.
    Act,
}

thread_local! {
    static ROLE: Cell<Role> = const { Cell::new(Role::Other) };
}

/// Name the calling thread's role (once, at the top of its loop).
pub fn set_role(r: Role) {
    ROLE.with(|c| c.set(r));
}

/// The calling thread's role.
#[must_use]
pub fn role() -> Role {
    ROLE.with(Cell::get)
}

/// A lock whose blocking waits are measured, by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockId {
    /// The device's GSP state (the drainer serves every privileged write under it).
    Gsp,
    /// One Translated channel's slot (a worker holds it across a whole pump).
    Slot,
    /// The plane's token → slot map.
    Slots,
    /// The Passthrough twins' map.
    Pt,
    /// The Passthrough engine-object index.
    PtObjs,
    /// The `(client, object)` → host token index.
    ByObj,
    /// The Translated channels' group/parent/device scopes.
    Scopes,
    /// The plane's capability counters.
    Caps,
    /// The Translated channels' software objects.
    SwObjs,
    /// The robust-channel events waiting for the drainer.
    RcQueue,
    /// The doorbell fast path's registry (held across `KVM_IOEVENTFD` by the main loop).
    DbReg,
    /// The act queue's sender.
    Acts,
    /// Any other plane map (groups, debugger sessions, CUDA limit, encoder sessions).
    Misc,
}

impl LockId {
    /// Every lock, in the order of [`Stall::locks`].
    pub const ALL: [LockId; 13] = [
        LockId::Gsp,
        LockId::Slot,
        LockId::Slots,
        LockId::Pt,
        LockId::PtObjs,
        LockId::ByObj,
        LockId::Scopes,
        LockId::Caps,
        LockId::SwObjs,
        LockId::RcQueue,
        LockId::DbReg,
        LockId::Acts,
        LockId::Misc,
    ];

    /// The name the status line prints.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            LockId::Gsp => "gsp",
            LockId::Slot => "slot",
            LockId::Slots => "slots",
            LockId::Pt => "pt",
            LockId::PtObjs => "pt_objs",
            LockId::ByObj => "by_obj",
            LockId::Scopes => "scopes",
            LockId::Caps => "caps",
            LockId::SwObjs => "sw_objs",
            LockId::RcQueue => "rc_queue",
            LockId::DbReg => "db_reg",
            LockId::Acts => "acts",
            LockId::Misc => "misc",
        }
    }

    const fn index(self) -> usize {
        self as usize
    }
}

/// The reply-hold ledger: when each currently held guest reply was first held, oldest first.
/// ⊘ Held replies are released strictly in order (`kf_gsp::GspFsm::release_held` stops at the
/// first still-pending one), so a length is enough to age them. Drainer only.
#[derive(Debug, Default)]
pub struct HeldBook {
    q: VecDeque<Instant>,
}

impl HeldBook {
    /// An empty ledger.
    #[must_use]
    pub const fn new() -> Self {
        HeldBook { q: VecDeque::new() }
    }

    /// The FSM now holds `held_len` replies: stamp the new ones `now`, and record the age of the
    /// released ones (the oldest) into `gauge`. Call after the holds of a pass were added and again
    /// after its releases, so a reply held and another released in one pass are not confused.
    pub fn sync(&mut self, held_len: usize, now: Instant, gauge: &MaxGauge) {
        while self.q.len() < held_len {
            self.q.push_back(now);
        }
        while self.q.len() > held_len {
            if let Some(t) = self.q.pop_front() {
                gauge.record(now.saturating_duration_since(t));
            }
        }
    }

    /// The age of the oldest reply held now, ns (0: none).
    #[must_use]
    pub fn oldest_ns(&self, now: Instant) -> u64 {
        self.q.front().map_or(0, |t| {
            u64::try_from(now.saturating_duration_since(*t).as_nanos()).unwrap_or(u64::MAX)
        })
    }

    /// Replies held now.
    #[must_use]
    pub fn len(&self) -> usize {
        self.q.len()
    }

    /// Whether none is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.q.is_empty()
    }
}

/// Every instrument of one device. Leaked for the process ([`Stall::leak`]) so a [`TimedMutex`]
/// holds a plain `&'static` and the hot path needs no `Arc`.
#[derive(Debug)]
pub struct Stall {
    /// One drainer iteration: loop top to the next park (or to the next iteration).
    pub drainer_pass: MaxGauge,
    /// One `apply_register`, entry to return.
    pub apply_register: MaxGauge,
    /// A held guest reply's age at release.
    pub held_reply_age: MaxGauge,
    /// An act's wait in the queue before its first step ran.
    pub act_queue_wait: MaxGauge,
    /// One synchronous act step.
    pub act_run: MaxGauge,
    /// An act's whole life, enqueue to resolution (waits included).
    pub act_total: MaxGauge,
    /// A drainer pass that doorbell servicing filled to its batch limit.
    pub doorbell_batch_full: AtomicU64,
    /// Blocked acquisitions of any [`TimedMutex`] / [`TimedRwLock`], any thread.
    pub lock_wait: MaxGauge,
    /// … by the drainer.
    pub drainer_lock_wait: MaxGauge,
    /// … by the act thread.
    pub act_lock_wait: MaxGauge,
    /// Per lock.
    pub locks: [MaxGauge; LockId::ALL.len()],
    /// Log lines dropped because the logger's queue was full.
    pub log_dropped: AtomicU64,
    /// Log lines handed to the logger.
    pub log_queued: AtomicU64,
}

impl Default for Stall {
    fn default() -> Self {
        Stall::new()
    }
}

impl Stall {
    /// A fresh set of instruments.
    #[must_use]
    pub fn new() -> Stall {
        Stall {
            drainer_pass: MaxGauge::new(),
            apply_register: MaxGauge::new(),
            held_reply_age: MaxGauge::new(),
            act_queue_wait: MaxGauge::new(),
            act_run: MaxGauge::new(),
            act_total: MaxGauge::new(),
            doorbell_batch_full: AtomicU64::new(0),
            lock_wait: MaxGauge::new(),
            drainer_lock_wait: MaxGauge::new(),
            act_lock_wait: MaxGauge::new(),
            locks: std::array::from_fn(|_| MaxGauge::new()),
            log_dropped: AtomicU64::new(0),
            log_queued: AtomicU64::new(0),
        }
    }

    /// A fresh set, leaked (process lifetime) — the device's, and a test's.
    #[must_use]
    pub fn leak() -> &'static Stall {
        Box::leak(Box::new(Stall::new()))
    }

    /// A blocked acquisition of lock `id` that waited `ns`.
    pub fn note_lock_wait(&self, id: LockId, ns: u64) {
        self.lock_wait.record_ns(ns);
        self.locks[id.index()].record_ns(ns);
        match role() {
            Role::Drainer => self.drainer_lock_wait.record_ns(ns),
            Role::Act => self.act_lock_wait.record_ns(ns),
            Role::Other => {}
        }
    }

    /// The lock with the longest wait: `(name, ns)`; `("-", 0)` when none ever blocked.
    #[must_use]
    pub fn worst_lock(&self) -> (&'static str, u64) {
        LockId::ALL
            .iter()
            .map(|id| (id.name(), self.locks[id.index()].max_ns()))
            .max_by_key(|(_, ns)| *ns)
            .filter(|(_, ns)| *ns > 0)
            .unwrap_or(("-", 0))
    }

    /// The status-line fragment (leading space included). `held` is the drainer's reply ledger,
    /// when the caller has it.
    #[must_use]
    pub fn fragment(&self, now: Instant, held: Option<&HeldBook>) -> String {
        let o = Ordering::Relaxed;
        let (worst, worst_ns) = self.worst_lock();
        let mut s = String::new();
        let _ = write!(
            s,
            " stall[drain_pass_max_us={} drain_pass_over10ms={} apply_max_us={} apply_over10ms={} held_age_max_us={} held_oldest_us={} act_wait_max_us={} act_run_max_us={} act_run_over10ms={} act_total_max_us={} lock_wait_max_us={} lock_wait_worst={}:{} drainer_lock_wait_max_us={} act_lock_wait_max_us={} doorbell_batch_full={} log_queued={} log_dropped={}]",
            self.drainer_pass.max_us(),
            self.drainer_pass.over(),
            self.apply_register.max_us(),
            self.apply_register.over(),
            self.held_reply_age.max_us(),
            held.map_or(0, |h| h.oldest_ns(now) / 1000),
            self.act_queue_wait.max_us(),
            self.act_run.max_us(),
            self.act_run.over(),
            self.act_total.max_us(),
            self.lock_wait.max_us(),
            worst,
            worst_ns / 1000,
            self.drainer_lock_wait.max_us(),
            self.act_lock_wait.max_us(),
            self.doorbell_batch_full.load(o),
            self.log_queued.load(o),
            self.log_dropped.load(o),
        );
        s
    }
}

/// A [`Mutex`] whose BLOCKED acquisitions are measured ([`Stall::note_lock_wait`]). The
/// uncontended path is one `try_lock`; the contended path times the blocking `lock`.
/// ⊘ Measurement only: it removes no wait. A lock the drainer must not wait on is removed from the
/// drainer's path, not wrapped.
#[derive(Debug)]
pub struct TimedMutex<T: ?Sized> {
    st: &'static Stall,
    id: LockId,
    m: Mutex<T>,
}

impl<T> TimedMutex<T> {
    /// A measured mutex named `id`, reporting to `st`.
    pub fn new(st: &'static Stall, id: LockId, v: T) -> Self {
        TimedMutex {
            st,
            id,
            m: Mutex::new(v),
        }
    }
}

impl<T: ?Sized> TimedMutex<T> {
    /// Acquire, recording the wait when it had to block.
    ///
    /// # Errors
    /// Poisoned, as [`Mutex::lock`].
    pub fn lock(&self) -> LockResult<MutexGuard<'_, T>> {
        match self.m.try_lock() {
            Ok(g) => Ok(g),
            Err(std::sync::TryLockError::Poisoned(p)) => Err(p),
            Err(std::sync::TryLockError::WouldBlock) => {
                let t = Instant::now();
                let r = self.m.lock();
                self.st.note_lock_wait(
                    self.id,
                    u64::try_from(t.elapsed().as_nanos()).unwrap_or(u64::MAX),
                );
                r
            }
        }
    }

    /// Acquire without blocking (never recorded: it never waits).
    ///
    /// # Errors
    /// Would block, or poisoned, as [`Mutex::try_lock`].
    pub fn try_lock(&self) -> TryLockResult<MutexGuard<'_, T>> {
        self.m.try_lock()
    }

    /// Whether a holder panicked, as [`Mutex::is_poisoned`].
    pub fn is_poisoned(&self) -> bool {
        self.m.is_poisoned()
    }
}

/// A [`RwLock`] whose blocked acquisitions are measured, as [`TimedMutex`].
#[derive(Debug)]
pub struct TimedRwLock<T: ?Sized> {
    st: &'static Stall,
    id: LockId,
    l: RwLock<T>,
}

impl<T> TimedRwLock<T> {
    /// A measured rwlock named `id`, reporting to `st`.
    pub fn new(st: &'static Stall, id: LockId, v: T) -> Self {
        TimedRwLock {
            st,
            id,
            l: RwLock::new(v),
        }
    }
}

impl<T: ?Sized> TimedRwLock<T> {
    /// Shared acquire, recording the wait when it had to block.
    ///
    /// # Errors
    /// Poisoned, as [`RwLock::read`].
    pub fn read(&self) -> LockResult<RwLockReadGuard<'_, T>> {
        match self.l.try_read() {
            Ok(g) => Ok(g),
            Err(std::sync::TryLockError::Poisoned(p)) => Err(p),
            Err(std::sync::TryLockError::WouldBlock) => {
                let t = Instant::now();
                let r = self.l.read();
                self.st.note_lock_wait(
                    self.id,
                    u64::try_from(t.elapsed().as_nanos()).unwrap_or(u64::MAX),
                );
                r
            }
        }
    }

    /// Exclusive acquire, recording the wait when it had to block.
    ///
    /// # Errors
    /// Poisoned, as [`RwLock::write`].
    pub fn write(&self) -> LockResult<RwLockWriteGuard<'_, T>> {
        match self.l.try_write() {
            Ok(g) => Ok(g),
            Err(std::sync::TryLockError::Poisoned(p)) => Err(p),
            Err(std::sync::TryLockError::WouldBlock) => {
                let t = Instant::now();
                let r = self.l.write();
                self.st.note_lock_wait(
                    self.id,
                    u64::try_from(t.elapsed().as_nanos()).unwrap_or(u64::MAX),
                );
                r
            }
        }
    }
}

/// Records the time from its creation to its drop into a gauge — for a scope with early returns.
pub struct Timed<'a> {
    gauge: &'a MaxGauge,
    t0: Instant,
}

impl<'a> Timed<'a> {
    /// Start timing into `gauge`.
    #[must_use]
    pub fn start(gauge: &'a MaxGauge) -> Self {
        Timed {
            gauge,
            t0: Instant::now(),
        }
    }
}

impl Drop for Timed<'_> {
    fn drop(&mut self) {
        self.gauge.since(self.t0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::mpsc;

    #[test]
    fn a_gauge_keeps_count_sum_max_and_the_stall_count() {
        let g = MaxGauge::new();
        g.record_ns(5_000);
        g.record_ns(STALL_NS);
        g.record_ns(7_000);
        assert_eq!(g.count(), 3);
        assert_eq!(g.max_ns(), STALL_NS);
        assert_eq!(g.max_us(), 10_000);
        assert_eq!(g.over(), 1, "only the sample AT the limit counts");
        assert_eq!(g.sum_ns(), 5_000 + STALL_NS + 7_000);
    }

    #[test]
    fn an_uncontended_lock_records_nothing_and_a_blocked_one_records_its_wait_by_name_and_role() {
        let st = Stall::leak();
        let m = Arc::new(TimedMutex::new(st, LockId::Pt, 0u32));
        drop(m.lock().unwrap());
        assert_eq!(st.lock_wait.count(), 0, "the fast path is a try_lock");
        let held = m.lock().unwrap();
        let (tx, rx) = mpsc::channel();
        let m2 = Arc::clone(&m);
        let h = std::thread::spawn(move || {
            set_role(Role::Drainer);
            let _ = tx.send(());
            drop(m2.lock().unwrap());
        });
        rx.recv().unwrap();
        std::thread::sleep(Duration::from_millis(40));
        drop(held);
        h.join().unwrap();
        assert_eq!(st.lock_wait.count(), 1);
        assert!(
            st.lock_wait.max_ns() >= 30_000_000,
            "the 40 ms hold is visible: {}",
            st.lock_wait.max_ns()
        );
        assert_eq!(st.drainer_lock_wait.count(), 1, "attributed to the drainer");
        assert_eq!(st.act_lock_wait.count(), 0);
        assert_eq!(st.worst_lock().0, "pt");
        assert_eq!(st.locks[LockId::Pt.index()].count(), 1);
        assert_eq!(st.locks[LockId::Slot.index()].count(), 0);
    }

    #[test]
    fn a_try_lock_never_waits_and_is_never_recorded() {
        let st = Stall::leak();
        let m = TimedMutex::new(st, LockId::Slot, ());
        let _g = m.lock().unwrap();
        assert!(m.try_lock().is_err());
        assert_eq!(st.lock_wait.count(), 0);
    }

    #[test]
    fn the_rwlock_records_a_blocked_writer() {
        let st = Stall::leak();
        let l = Arc::new(TimedRwLock::new(st, LockId::Slots, 0u8));
        let r = l.read().unwrap();
        let l2 = Arc::clone(&l);
        let h = std::thread::spawn(move || {
            *l2.write().unwrap() = 1;
        });
        std::thread::sleep(Duration::from_millis(30));
        drop(r);
        h.join().unwrap();
        assert_eq!(st.locks[LockId::Slots.index()].count(), 1);
        assert!(st.lock_wait.max_ns() >= 20_000_000);
        assert_eq!(*l.read().unwrap(), 1);
    }

    #[test]
    fn the_held_book_ages_replies_oldest_first_on_an_injected_clock() {
        let g = MaxGauge::new();
        let t0 = Instant::now();
        let mut b = HeldBook::new();
        b.sync(2, t0, &g);
        assert_eq!(b.len(), 2);
        // 50 ms later one is released and a new one is held in the same pass: sync the hold first.
        let t1 = t0 + Duration::from_millis(50);
        b.sync(3, t1, &g);
        b.sync(2, t1, &g);
        assert_eq!(g.count(), 1);
        assert_eq!(g.max_ns(), 50_000_000, "the OLDEST was released");
        assert_eq!(
            b.oldest_ns(t1),
            50_000_000,
            "the second reply of the first batch"
        );
        let t2 = t0 + Duration::from_millis(80);
        b.sync(0, t2, &g);
        assert_eq!(g.count(), 3);
        assert_eq!(g.max_ns(), 80_000_000);
        assert!(b.is_empty());
        assert_eq!(b.oldest_ns(t2), 0);
    }

    /// ★ The counters the status line carries are NAMED, and a normal boot prints them with no flag.
    #[test]
    fn the_fragment_names_every_instrument_and_moves_with_them() {
        let st = Stall::leak();
        let now = Instant::now();
        let quiet = st.fragment(now, None);
        for name in [
            "drain_pass_max_us=0",
            "apply_max_us=0",
            "held_age_max_us=0",
            "held_oldest_us=0",
            "act_wait_max_us=0",
            "act_run_max_us=0",
            "lock_wait_max_us=0",
            "lock_wait_worst=-:0",
            "drainer_lock_wait_max_us=0",
            "act_lock_wait_max_us=0",
            "doorbell_batch_full=0",
            "log_dropped=0",
        ] {
            assert!(quiet.contains(name), "{name} missing from {quiet}");
        }
        st.drainer_pass.record_ns(1_234_000);
        st.apply_register.record_ns(2_000_000);
        st.held_reply_age.record_ns(3_000_000);
        st.act_queue_wait.record_ns(4_000_000);
        st.act_run.record_ns(15_000_000);
        st.note_lock_wait(LockId::Gsp, 6_000_000);
        st.log_dropped.fetch_add(7, Ordering::Relaxed);
        let mut book = HeldBook::new();
        book.sync(1, now, &MaxGauge::new());
        let loud = st.fragment(now + Duration::from_millis(9), Some(&book));
        for name in [
            "drain_pass_max_us=1234",
            "apply_max_us=2000",
            "held_age_max_us=3000",
            "held_oldest_us=9000",
            "act_wait_max_us=4000",
            "act_run_max_us=15000",
            "act_run_over10ms=1",
            "lock_wait_max_us=6000",
            "lock_wait_worst=gsp:6000",
            "log_dropped=7",
        ] {
            assert!(loud.contains(name), "{name} missing from {loud}");
        }
    }

    #[test]
    fn a_timed_scope_records_on_every_exit() {
        let g = MaxGauge::new();
        let f = |early: bool| {
            let _t = Timed::start(&g);
            if early {
                return 1;
            }
            2
        };
        f(true);
        f(false);
        assert_eq!(g.count(), 2);
    }
}
