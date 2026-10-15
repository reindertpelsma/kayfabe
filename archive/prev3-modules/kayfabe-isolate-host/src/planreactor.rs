//! ★★★★★ **The plan reactor — in-flight work bounded by OUTSTANDING PLANS, not by the
//! caller's thread count** (owner goal 8, 2026-09-13: *"TWO raw clients with the mean test in
//! parallel"*).
//!
//! # 1. The shape this replaces, measured
//!
//! A host RM verb is a synchronous write-then-blocking-read over one unix socket:
//! [`crate::isolate::ProxyRmBackend::call_inner`] writes the frame and then parks the
//! calling thread inside `read_frame` until the child answers. Every production caller
//! reaches that through [`kayfabe_isolate::Worker::execute`], which runs a whole **verb
//! chain** — for `VerbPlan::PinGuestRam`, three consecutive round trips — on the calling
//! thread.
//!
//! Off the vCPU there is exactly **one** such thread in the shim
//! (`kayfabe-doorbell-publish`, a single `while let Some(job) = queue.take_blocking()`
//! loop), so the off-trap verb path is **totally ordered**: while it is parked in one
//! isolate's `read`, no other isolate's work can even be started. Two guest processes are
//! two isolates, two child processes and two RM clients — genuinely independent on the host
//! — and they were being funnelled through a single serialising lane.
//!
//! # 2. ⊘ WHAT THIS IS NOT, AND THE MEASUREMENTS THAT SAY SO
//!
//! ⊘ **It is not a throughput fix, and the request-level epoll demultiplexer it might have
//! been would not be one either.** Three numbers already in this tree:
//!
//! - `[measured w394e, real GA106]` (`docs/design/w394_cuda_apps_and_the_parity_gate.md`
//!   §"VERBCOST OVERTURNS MY OWN COST MODEL"): of a 234 ms CUDA launch, **host verbs are
//!   7.15 ms — 3.1 %**. Our own trap-side page-table decode and publication are 96.9 %.
//! - `[measured w321, real GA106]` (`docs/design/the_drain_cost_is_per_call_not_per_page.md`):
//!   inside one round trip the split is **transport ≈ 29 µs (39 %) / RM ioctl ≈ 132 µs
//!   (61 %)**. Making the transport non-blocking cannot remove the 61 %.
//! - `[measured R12, real GA106]` (`tests/real_isolate.rs` module docs, 800 alloc+free
//!   pairs): 1 worker sequential **1610 ms**, 1 isolate × 4 workers **1602 ms**, 4 isolates
//!   × 1 worker **1610 ms** — **1.00x for every arrangement**. RM holds the device-global
//!   API lock in WRITE across the GSP RPC, so for the alloc/free class *no* concurrency
//!   scheme buys throughput.
//!
//! ⇒ The property that is actually missing is **liveness isolation between clients**, not
//! parallel ioctls: one isolate's long verb must not stall another isolate's short one. The
//! longest verb in the surface is [`kayfabe_isolate::RmBackend::ce_copy`], which waits on a
//! real GPU semaphore up to `crate::rm::CE_COPY_TIMEOUT` = **2 s**, polling at 1 ms — a
//! second client parked behind that has no way to make progress at all.
//!
//! # 3. Why the unit is the PLAN and not the REQUEST
//!
//! An epoll demultiplexer over worker sockets would let one thread hold K *requests* open.
//! It would buy nothing here, because the thing a caller submits is a **chain**: `execute`
//! feeds each verb's reply into the next verb's arguments, in 700 lines of hand-written
//! unwind. Interleaving two chains on one thread needs the chain to be *resumable* — a state
//! machine over every `VerbPlan` variant — and interleaving them is the only way a
//! request-level demux delivers anything to an existing caller.
//!
//! So the async unit here is the whole plan. `execute` is called **verbatim**, on a lane
//! thread, with every gate it already has (R1 `assert_lock_free`, the `OffTrap` witness, the
//! foreign-handle check). Nothing about the protocol, the framing or the reply matching
//! changes — which matters, because of §4.
//!
//! # 4. ★★★ THE CONSTRAINT THAT FORBIDS THE OBVIOUS DESIGN: REPLIES ARE MATCHED BY ARRIVAL
//! ORDER
//!
//! [`crate::proto::Reply`] carries **no request id and no txn**. `Envelope::txn` travels
//! outbound only, and `proto`'s own module docs say why: each pool worker owns its own
//! socket, the channel is 1-deep, *"there is no demux, no pending list, and no `txn_id` to
//! confuse"*. The match of a reply to its request is therefore **positional**, and it is
//! sound only because exactly one request can be outstanding on a socket — a fact the borrow
//! checker enforces, since the socket is reached only through a `&mut Worker`.
//!
//! ⇒ **Any future design that puts more than one request on one socket is silent
//! cross-transaction corruption**, not a wrong answer: reply N is decoded into request
//! N+1's caller. It would need a request id in [`crate::proto::Reply`] first. This module
//! keeps the 1-deep invariant by *moving the `Worker` in*: while a plan is in flight the
//! reactor owns it, so no second submitter can exist.
//!
//! ★ Second constraint, same family: **four replies carry a descriptor** (`ExportBacking`,
//! `ExportUsermodeView`, `JoinFbLeaf`) and are read by `read_frame_with_fds` with an
//! `max_fds` allowance, while every other reply is read by `read_frame`, which has **no
//! control buffer at all**. A reply taken by the wrong reader has its descriptor *silently
//! dropped by the kernel* (`proto.rs`, `Reply::UsermodeView`'s own warning). The allowance
//! is a property of the **request**, so a generic demultiplexer cannot choose it from the
//! frame — it would have to carry per-pending state saying which reader to use. Here the
//! question does not arise: the reader is chosen inside `execute`, as it always was.
//!
//! # 5. Ordering — what the caller keeps and what it must now establish
//!
//! Today's single funnel gives a **total order** over every off-trap verb plan. This reactor
//! weakens that, and the weakening is the point. [`LanePolicy`] names exactly how far:
//!
//! - [`LanePolicy::PerIsolate`] (the default) — one lane thread per [`IsolateId`]. Plans on
//!   one isolate stay in **submission order**; plans on different isolates run concurrently.
//!   Cross-isolate reordering is safe by the tree's own boundary: a different isolate is a
//!   different child process, a different RM client namespace and a different handle space
//!   (`l1_concurrency.md` §12.26), so no two isolates' plans can name the same host object.
//! - [`LanePolicy::PerWorker`] — one lane per `(IsolateId, WorkerId)`. More in flight, and
//!   **it reorders plans within one isolate**, which nothing in the core has established as
//!   safe. It exists so the question can be *measured* rather than argued; it is not the
//!   default and must not become one without a falsifier.
//!
//! # 6. What the caller owes
//!
//! A [`PlanDone`] carries the checked-out [`Worker`] back. **Dropping one wedges that pool
//! slot forever** — the isolate never quiesces, so its proc is deferred at every quiesce
//! point. Hence `#[must_use]` on both `PlanDone` and [`Rejected`], and hence
//! [`PlanReactor`]'s `Drop` counting and *printing* anything it had to abandon rather than
//! letting it vanish.

use std::collections::{BTreeMap, VecDeque};
use std::os::fd::BorrowedFd;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use kayfabe_isolate::{IsolateId, VerbFailure, VerbPlan, VerbReply, Worker, WorkerId};
use kayfabe_linux_raw::{Notifier, RawError};

/// How many lane threads one reactor will create before refusing.
///
/// ⊘ A bound, not a sizing: the real bound is the number of live isolates, which the core
/// controls (one per `(Proc, GpuId)`). This exists so a guest that could drive isolate
/// creation cannot drive *our* thread creation with it, and so the refusal has a name
/// instead of an `ENOMEM` from `thread::spawn`.
pub const DEFAULT_LANE_CAP: usize = 64;

/// ★ Completions **abandoned** by a reactor that was dropped without being drained, process
/// wide and monotonic.
///
/// ⊘ A global, deliberately: it is read *after* the reactor that produced it no longer
/// exists, so it cannot live on the reactor. Monotonic and never reset, for `ipc_totals`'
/// reason — a resettable counter lets two readers steal each other's interval.
///
/// Each unit is a checked-out `Worker` that was never checked in, i.e. a permanently wedged
/// pool slot. **A clean run leaves this at zero**; anything else is a leak with a number.
static ABANDONED_TOTAL: AtomicU64 = AtomicU64::new(0);

/// Read [`ABANDONED_TOTAL`].
#[must_use]
pub fn abandoned_completions_total() -> u64 {
    ABANDONED_TOTAL.load(Ordering::Relaxed)
}

/// How plans are assigned to lane threads — i.e. exactly which reorderings become possible.
/// See the module docs §5 before changing a caller from the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanePolicy {
    /// One lane per isolate: per-isolate order preserved, cross-isolate concurrency.
    PerIsolate,
    /// One lane per pool worker: more concurrency, **and intra-isolate reordering**.
    PerWorker,
}

/// A lane's identity. `worker` is `None` under [`LanePolicy::PerIsolate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct LaneKey {
    isolate: IsolateId,
    worker: Option<WorkerId>,
}

/// One finished plan: **the worker comes back with it.**
///
/// Dropping this without checking the worker back in wedges that pool slot for the life of
/// the isolate — see the module docs §6.
#[must_use = "a PlanDone carries a checked-out Worker; dropping it wedges that pool slot \
              forever, so the isolate never quiesces. Check it back in."]
#[derive(Debug)]
pub struct PlanDone {
    /// The cookie the submitter passed.
    pub cookie: u64,
    /// Which isolate served it.
    pub isolate: IsolateId,
    /// The worker, to be checked back in.
    pub worker: Worker,
    /// Exactly what [`Worker::execute`] returned — no wrapping, no reinterpretation.
    pub outcome: Result<VerbReply, VerbFailure>,
}

impl PlanDone {
    /// Take it apart. The `Worker` still has to be checked in.
    #[must_use = "the Worker in this tuple still has to be checked back in"]
    pub fn into_parts(self) -> (u64, Worker, Result<VerbReply, VerbFailure>) {
        (self.cookie, self.worker, self.outcome)
    }
}

/// Why a submission was refused — **carrying the worker back**, because a refusal that ate
/// the worker would wedge the slot exactly as a dropped [`PlanDone`] does.
#[must_use = "a Rejected carries the Worker that was NOT submitted; check it back in."]
#[derive(Debug)]
pub struct Rejected {
    /// The worker, untouched. Nothing was written to its socket.
    pub worker: Worker,
    /// Why.
    pub why: RejectReason,
}

/// The refusals [`PlanReactor::submit`] can produce. Each is a *named* state, never a
/// silent fallback to running the plan inline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectReason {
    /// This reactor already runs `cap` lanes and this plan needs a new one.
    LaneCapExceeded {
        /// The cap that bound.
        cap: usize,
    },
    /// The lane thread could not be created; carries the OS's own words.
    SpawnFailed(String),
    /// [`PlanReactor::shutdown`] has run. The reactor accepts nothing further.
    Stopped,
}

/// A reactor's counters. Every field is incremented by this module and read by
/// [`PlanReactor::census`] and by `tests/plan_reactor.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanReactorStats {
    /// Plans accepted.
    pub submitted: u64,
    /// Plans whose `execute` returned (successfully or not) and were queued for the caller.
    pub completed: u64,
    /// Accepted minus drained: plans the caller has not collected yet.
    pub in_flight: usize,
    /// The high-water mark of `in_flight`. ★ **This is the number goal 8 is about**: it is
    /// how many plans were open at once, and it exceeding 1 is the whole claim.
    pub peak_in_flight: usize,
    /// Lane threads created over this reactor's life.
    pub lanes_spawned: u64,
    /// Lane threads alive now.
    pub lanes_live: usize,
    /// Submissions refused, by any [`RejectReason`].
    pub rejected: u64,
    /// Completions the reactor had to abandon at `Drop` — **each one is a wedged pool
    /// slot**. Must be zero in a clean teardown.
    pub abandoned: u64,
    /// `Notifier::signal` failures (a saturated 64-bit counter: nobody drained in 2^64
    /// signals). Reported, never retried — a retry loop is a blocking call wearing a
    /// non-blocking API.
    pub signal_failures: u64,
}

#[derive(Debug, Default)]
struct Stats {
    submitted: AtomicU64,
    completed: AtomicU64,
    in_flight: AtomicUsize,
    peak_in_flight: AtomicUsize,
    lanes_spawned: AtomicU64,
    rejected: AtomicU64,
    abandoned: AtomicU64,
    signal_failures: AtomicU64,
}

/// The completion side, shared with every lane thread.
#[derive(Debug)]
struct Shared {
    done: Mutex<VecDeque<PlanDone>>,
    /// For [`PlanReactor::wait`] — a caller with no readiness loop of its own.
    wake: Condvar,
    /// For [`PlanReactor::readiness`] — a caller that already owns an epoll set.
    ///
    /// ⊘ Both, and neither is redundant: the shim's publication worker blocks on a condvar
    /// today, while its completion observer blocks on a `Poller`. A reactor that offered
    /// only one of the two would force the wiring to rebuild the other caller's loop.
    notify: Notifier,
    stats: Stats,
}

impl Shared {
    /// Push a completion and ring both doors. **Neither ring happens under the queue lock**:
    /// `Notifier::signal` is a syscall, and the rule against a syscall under a lock has no
    /// exception for a small one.
    fn complete(&self, done: PlanDone) {
        // ⊘ **The counter moves BEFORE the item is visible, and the order is load-bearing.**
        // Bumped after the push, a collector can drain the completion and read `completed`
        // still at its old value — so `stats()` would under-report work the caller already
        // holds. `[measured 2026-09-13]` that is not theoretical: it failed
        // `one_thread_holds_two_isolates_parked_at_once` once in a parallel `cargo test`
        // run and never in a serial one, which is exactly how this class hides. This order
        // can only ever over-report by the width of one push.
        self.stats.completed.fetch_add(1, Ordering::Relaxed);
        {
            let mut q = self.done.lock().unwrap_or_else(|e| e.into_inner());
            q.push_back(done);
        }
        self.wake.notify_all();
        if self.notify.signal().is_err() {
            self.stats.signal_failures.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[derive(Debug)]
struct LaneState {
    queue: VecDeque<Job>,
    stopping: bool,
}

#[derive(Debug)]
struct LaneQueue {
    state: Mutex<LaneState>,
    wake: Condvar,
}

#[derive(Debug)]
struct Job {
    cookie: u64,
    isolate: IsolateId,
    worker: Worker,
    plan: VerbPlan,
}

struct LaneHandle {
    queue: Arc<LaneQueue>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl core::fmt::Debug for LaneHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LaneHandle")
            .field("running", &self.join.is_some())
            .finish()
    }
}

/// ★★★ The reactor: submit a plan with a worker, collect it later.
///
/// Read the module docs before wiring one — especially §5 (what ordering the caller loses)
/// and §6 (a dropped [`PlanDone`] is a wedged pool slot).
#[derive(Debug)]
pub struct PlanReactor {
    shared: Arc<Shared>,
    lanes: Mutex<BTreeMap<LaneKey, LaneHandle>>,
    policy: LanePolicy,
    cap: usize,
    stopped: std::sync::atomic::AtomicBool,
}

impl PlanReactor {
    /// A reactor with [`LanePolicy::PerIsolate`] and [`DEFAULT_LANE_CAP`].
    ///
    /// # Errors
    /// The notify descriptor could not be created.
    ///
    /// # Panics
    /// If this thread holds any ranked lock (R1) — `Notifier::create` asserts it.
    pub fn new() -> Result<Self, RawError> {
        Self::with_policy(LanePolicy::PerIsolate, DEFAULT_LANE_CAP)
    }

    /// A reactor with an explicit policy and lane cap.
    ///
    /// # Errors
    /// The notify descriptor could not be created.
    ///
    /// # Panics
    /// If this thread holds any ranked lock (R1).
    pub fn with_policy(policy: LanePolicy, cap: usize) -> Result<Self, RawError> {
        Ok(PlanReactor {
            shared: Arc::new(Shared {
                done: Mutex::new(VecDeque::new()),
                wake: Condvar::new(),
                notify: Notifier::create()?,
                stats: Stats::default(),
            }),
            lanes: Mutex::new(BTreeMap::new()),
            policy,
            cap,
            stopped: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// The descriptor a caller adds to its own readiness set. Readable ⇒ at least one
    /// completion is queued; [`PlanReactor::drain`] clears it.
    #[must_use]
    pub fn readiness(&self) -> BorrowedFd<'_> {
        self.shared.notify.as_source_fd()
    }

    /// This reactor's lane policy.
    #[must_use]
    pub fn policy(&self) -> LanePolicy {
        self.policy
    }

    /// ★ **Submit `plan` to be run on `worker`, off this thread.**
    ///
    /// The worker is **moved in** — that is what keeps the socket 1-deep and therefore keeps
    /// the positional reply match sound (module docs §4). It comes back in the
    /// [`PlanDone`], or immediately inside [`Rejected`].
    ///
    /// # Errors
    /// [`Rejected`], carrying the worker untouched. Nothing has been written to its socket.
    ///
    /// # Panics
    /// If this thread holds any ranked lock (R1). Submitting may create a thread, and it
    /// hands a worker to a thread that is about to issue a host verb; both are the
    /// blocking-under-a-lock this tree asserts against at every other door.
    pub fn submit(&self, worker: Worker, plan: VerbPlan, cookie: u64) -> Result<(), Rejected> {
        kayfabe_util::lockwitness::assert_lock_free("submitting a verb plan to the plan reactor");
        if self.stopped.load(Ordering::Acquire) {
            self.shared.stats.rejected.fetch_add(1, Ordering::Relaxed);
            return Err(Rejected {
                worker,
                why: RejectReason::Stopped,
            });
        }
        let key = LaneKey {
            isolate: worker.isolate(),
            worker: match self.policy {
                LanePolicy::PerIsolate => None,
                LanePolicy::PerWorker => Some(worker.id()),
            },
        };
        let queue = match self.lane(key) {
            Ok(q) => q,
            Err(why) => {
                self.shared.stats.rejected.fetch_add(1, Ordering::Relaxed);
                return Err(Rejected { worker, why });
            }
        };
        let job = Job {
            cookie,
            isolate: key.isolate,
            worker,
            plan,
        };
        {
            let mut st = queue.state.lock().unwrap_or_else(|e| e.into_inner());
            st.queue.push_back(job);
        }
        self.shared.stats.submitted.fetch_add(1, Ordering::Relaxed);
        let now = self.shared.stats.in_flight.fetch_add(1, Ordering::Relaxed) + 1;
        self.shared
            .stats
            .peak_in_flight
            .fetch_max(now, Ordering::Relaxed);
        queue.wake.notify_one();
        Ok(())
    }

    /// Get this lane's queue, creating the lane if it does not exist.
    ///
    /// ⊘ **The thread is created with the lane map UNLOCKED**, then adopted under the lock.
    /// `thread::spawn` allocates a stack and enters the kernel; doing it under a lock every
    /// other submitter needs is the shape this tree keeps paying for. A losing racer stops
    /// its own spare thread and uses the winner's.
    fn lane(&self, key: LaneKey) -> Result<Arc<LaneQueue>, RejectReason> {
        {
            let lanes = self.lanes.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(l) = lanes.get(&key) {
                return Ok(Arc::clone(&l.queue));
            }
            if lanes.len() >= self.cap {
                return Err(RejectReason::LaneCapExceeded { cap: self.cap });
            }
        }
        let queue = Arc::new(LaneQueue {
            state: Mutex::new(LaneState {
                queue: VecDeque::new(),
                stopping: false,
            }),
            wake: Condvar::new(),
        });
        let shared = Arc::clone(&self.shared);
        let lane_queue = Arc::clone(&queue);
        let join = std::thread::Builder::new()
            .name(format!(
                "kayfabe-plan-lane-p{}g{}",
                key.isolate.proc(),
                key.isolate.gpu().0
            ))
            .spawn(move || lane_loop(&lane_queue, &shared))
            .map_err(|e| RejectReason::SpawnFailed(e.to_string()))?;

        let mut lanes = self.lanes.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = lanes.get(&key) {
            // Someone else won. Stop the spare we just made, and use theirs.
            let winner = Arc::clone(&existing.queue);
            {
                let mut st = queue.state.lock().unwrap_or_else(|e| e.into_inner());
                st.stopping = true;
            }
            queue.wake.notify_all();
            drop(lanes);
            let _ = join.join();
            return Ok(winner);
        }
        if lanes.len() >= self.cap {
            // The cap was reached while we were spawning. Refuse, and take our thread with
            // us rather than leaving it parked.
            {
                let mut st = queue.state.lock().unwrap_or_else(|e| e.into_inner());
                st.stopping = true;
            }
            queue.wake.notify_all();
            drop(lanes);
            let _ = join.join();
            return Err(RejectReason::LaneCapExceeded { cap: self.cap });
        }
        lanes.insert(
            key,
            LaneHandle {
                queue: Arc::clone(&queue),
                join: Some(join),
            },
        );
        self.shared
            .stats
            .lanes_spawned
            .fetch_add(1, Ordering::Relaxed);
        Ok(queue)
    }

    /// Collect every completion that has arrived. Never blocks.
    ///
    /// ⊘ The notify counter is drained **first** and the queue second. A completion landing
    /// between the two leaves the counter armed for a wake that finds nothing — an
    /// over-report. The other order loses it: the queue would be empty when taken and the
    /// counter already clear.
    ///
    /// # Panics
    /// If this thread holds any ranked lock (R1) — `Notifier::drain` asserts it.
    pub fn drain(&self) -> Vec<PlanDone> {
        let _ = self.shared.notify.drain();
        let mut q = self.shared.done.lock().unwrap_or_else(|e| e.into_inner());
        let out: Vec<PlanDone> = q.drain(..).collect();
        drop(q);
        self.shared
            .stats
            .in_flight
            .fetch_sub(out.len(), Ordering::Relaxed);
        out
    }

    /// Block until at least one completion is available or `within` elapses, then collect.
    ///
    /// For a caller with no readiness loop of its own. An empty result means the bound
    /// elapsed — it is a timeout, not "nothing is in flight".
    ///
    /// # Panics
    /// If this thread holds any ranked lock (R1).
    pub fn wait(&self, within: Duration) -> Vec<PlanDone> {
        kayfabe_util::lockwitness::assert_lock_free("waiting on the plan reactor");
        {
            let q = self.shared.done.lock().unwrap_or_else(|e| e.into_inner());
            let _unused = self
                .shared
                .wake
                .wait_timeout_while(q, within, |q| q.is_empty())
                .unwrap_or_else(|e| e.into_inner());
        }
        self.drain()
    }

    /// Plans accepted and not yet collected.
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.shared.stats.in_flight.load(Ordering::Relaxed)
    }

    /// This reactor's counters.
    #[must_use]
    pub fn stats(&self) -> PlanReactorStats {
        let s = &self.shared.stats;
        PlanReactorStats {
            submitted: s.submitted.load(Ordering::Relaxed),
            completed: s.completed.load(Ordering::Relaxed),
            in_flight: s.in_flight.load(Ordering::Relaxed),
            peak_in_flight: s.peak_in_flight.load(Ordering::Relaxed),
            lanes_spawned: s.lanes_spawned.load(Ordering::Relaxed),
            lanes_live: self.lanes.lock().unwrap_or_else(|e| e.into_inner()).len(),
            rejected: s.rejected.load(Ordering::Relaxed),
            abandoned: s.abandoned.load(Ordering::Relaxed),
            signal_failures: s.signal_failures.load(Ordering::Relaxed),
        }
    }

    /// One line for a boot report.
    #[must_use]
    pub fn census(&self) -> String {
        let s = self.stats();
        if s.submitted == 0 {
            // ⊘ Nothing was ever submitted is a fact about the boot, not a tidy zero row.
            return "PLANREACTOR ⊘ NO PLAN WAS EVER SUBMITTED — unmeasured, not zero".to_owned();
        }
        format!(
            "PLANREACTOR policy={:?} submitted={} completed={} in_flight={} peak_in_flight={} \
             lanes={}/{} rejected={} abandoned={} signal_failures={} ⊘ peak_in_flight>1 is the \
             claim: that many plans were open at once on ONE caller thread",
            self.policy,
            s.submitted,
            s.completed,
            s.in_flight,
            s.peak_in_flight,
            s.lanes_live,
            s.lanes_spawned,
            s.rejected,
            s.abandoned,
            s.signal_failures,
        )
    }

    /// ★ Stop every lane and hand back everything that finished.
    ///
    /// In-flight plans are **not** cancelled: a host verb has no abort, and abandoning one
    /// would leak whatever it allocated. Each lane finishes the job it is on, drains its
    /// queue, and exits; this joins them all and returns every completion, so no `Worker` is
    /// lost. Cancellation, if wanted, is the isolate's own out-of-band path
    /// (`Isolate::cancel_handle`) and is unchanged by this module.
    ///
    /// # Panics
    /// If this thread holds any ranked lock (R1) — joining a thread blocks.
    pub fn shutdown(&self) -> Vec<PlanDone> {
        kayfabe_util::lockwitness::assert_lock_free("shutting the plan reactor down");
        self.stopped.store(true, Ordering::Release);
        let handles: Vec<LaneHandle> = {
            let mut lanes = self.lanes.lock().unwrap_or_else(|e| e.into_inner());
            core::mem::take(&mut *lanes).into_values().collect()
        };
        for h in &handles {
            {
                let mut st = h.queue.state.lock().unwrap_or_else(|e| e.into_inner());
                st.stopping = true;
            }
            h.queue.wake.notify_all();
        }
        for mut h in handles {
            if let Some(j) = h.join.take() {
                let _ = j.join();
            }
        }
        self.drain()
    }
}

impl Drop for PlanReactor {
    /// ⚠ **Loud, because the alternative is invisible.** Anything still queued at drop is a
    /// checked-out `Worker` that will never be checked in — a wedged pool slot and an
    /// isolate that never quiesces. A silent drop would make that unfindable.
    fn drop(&mut self) {
        let left = self.shutdown();
        if left.is_empty() {
            return;
        }
        let n = left.len();
        self.shared
            .stats
            .abandoned
            .fetch_add(n as u64, Ordering::Relaxed);
        ABANDONED_TOTAL.fetch_add(n as u64, Ordering::Relaxed);
        eprintln!(
            "kayfabe: PLANREACTOR ⚠ DROPPED WITH {n} UNCOLLECTED COMPLETION(S) — that many \
             checked-out Workers are now lost and their pool slots are wedged for the life of \
             their isolates. Call `shutdown()` and check them in."
        );
    }
}

/// One lane's body: take a job, run its chain, post the completion.
///
/// ⊘ It holds **no lock** while `execute` runs. The queue lock is taken to pop and released
/// before the verb; the completion lock is taken to push and released before the wake. That
/// is the whole of R1 for this thread, and `Worker::execute` re-asserts it anyway.
fn lane_loop(queue: &Arc<LaneQueue>, shared: &Arc<Shared>) {
    // ★ Blocking here is the design working (owner, 2026-09-09) — this thread exists to be
    // the one that waits, so the lock-cost census must not read it as a violation.
    kayfabe_util::lock::declare_thread_class(kayfabe_util::lock::ThreadClass::Worker);
    loop {
        let job = {
            let mut st = queue.state.lock().unwrap_or_else(|e| e.into_inner());
            while st.queue.is_empty() && !st.stopping {
                st = queue.wake.wait(st).unwrap_or_else(|e| e.into_inner());
            }
            match st.queue.pop_front() {
                Some(j) => j,
                // Stopping AND empty: nothing is owed.
                None => return,
            }
        };
        let Job {
            cookie,
            isolate,
            mut worker,
            plan,
        } = job;
        // ★ Minted HERE and not carried in: `OffTrap` is `!Send` precisely so a witness
        // cannot be posted to another thread (`kayfabe-isolate/tests/ui/
        // send_a_trap_witness_to_another_thread.rs`). A lane thread is never inside a guest
        // trap, so this takes the honest `claim` branch and bumps `off_trap_claims`.
        let off = kayfabe_util::trapwitness::OffTrap::claim(
            "kayfabe-isolate-host::PlanReactor — a submitted verb chain",
        );
        let outcome = worker.execute(&plan, &off);
        shared.complete(PlanDone {
            cookie,
            isolate,
            worker,
            outcome,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_reactor_says_it_was_never_asked_rather_than_printing_zeros() {
        let r = PlanReactor::new().expect("notify descriptor");
        assert!(
            r.census().contains("NO PLAN WAS EVER SUBMITTED"),
            "{}",
            r.census()
        );
        let s = r.stats();
        assert_eq!((s.submitted, s.completed, s.lanes_spawned), (0, 0, 0));
        assert_eq!(s.in_flight, 0);
    }

    #[test]
    fn drain_on_an_idle_reactor_is_empty_and_does_not_block() {
        let r = PlanReactor::new().expect("notify descriptor");
        assert!(r.drain().is_empty());
        assert!(r.wait(Duration::from_millis(1)).is_empty());
    }

    #[test]
    fn the_two_policies_key_lanes_differently() {
        // The keys are private, so assert the behaviour that distinguishes them through the
        // one observable that does not need an isolate: `policy()`.
        assert_eq!(
            PlanReactor::with_policy(LanePolicy::PerWorker, 4)
                .expect("notify")
                .policy(),
            LanePolicy::PerWorker
        );
        assert_eq!(
            PlanReactor::new().expect("notify").policy(),
            LanePolicy::PerIsolate
        );
    }
}
