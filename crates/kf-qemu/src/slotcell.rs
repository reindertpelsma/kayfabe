//! ★★★ **The Translated channel's slot, split by who may touch which half** — so the register
//! drainer never takes the lock a worker holds across a whole pump
//! (`docs/design/V3_NONSTALL_THREADS.md` §3.A).
//!
//! A [`SlotCell`] is two things:
//!
//! - [`SlotMeta`] — the few facts the DRAINER needs about a channel while it serves the guest's RPCs:
//!   its guest token (`guest_idx`, immutable), its channel group (`tsg`, immutable), and two flags
//!   (`scheduled`, `stopped`) that are atomics. The drainer reads and writes ONLY these, with no lock.
//! - the pump state `T` (`Slot` in the device) behind a [`TimedMutex`] — held by a worker across a
//!   whole `pump` (a CPU-view arm + `mmap`, a USERD scan, the completion probe), and by the act
//!   thread. ⊘ The drainer never takes it: a drainer that waited here would stall every guest RPC
//!   (owner rule 2026-10-09).
//!
//! ## What the atomic flags change, exactly
//!
//! `scheduled` and `stopped` used to be written under the slot lock, so a `GPFIFO_SCHEDULE(false)`
//! statement waited for any pump in flight. Now the flag flips at once and the pump checks it at its
//! start (`ChanPlane::serve`): a pump already past the check finishes that one pass, as the GPU
//! finishes work it has already fetched when a channel is descheduled. ⊘ The pump authors `GP_GET`
//! only for work the host fence reached, so no completion is forged by the one extra pass. The FREE
//! path is unchanged in what it guarantees: it takes the lock itself, on the act thread, before it
//! frees the twin.
//!
//! Before this change the drainer's `try_lock` paths had a second defect: under contention
//! `try_lock().is_ok_and(|g| g.stopped)` read "not stopped" and a free's
//! `try_lock … g.scheduled = false` silently did nothing, so a stopped channel was re-scheduled as
//! if running, and a freed channel's pump kept running. Atomics have no contended state.

use kf_chan::actloop::{Cont, Step, Wait};
use kf_chan::stall::{LockId, Stall, TimedMutex};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockResult};

/// The host ring's `(GR context, video context)`, as `HostRing` reports them.
pub type HostContexts = (Option<(u32, u32)>, Option<(u32, u32, u32)>);

/// What the drainer AND the act thread may know about a channel without its pump lock.
///
/// Immutable after birth: [`Self::guest_idx`], [`Self::tsg`], [`Self::host`],
/// [`Self::guest_engine`], [`Self::gr_context`], [`Self::video_context`] (the host ring's
/// contexts are fixed at its birth).
/// Atomic: the four flags. [`Self::ctx`] is written by the act thread only (and read by the probe),
/// so its mutex is never contended.
#[derive(Debug)]
pub struct SlotMeta {
    /// The guest's token (the token-table index, the guest's chid). Immutable.
    pub guest_idx: u32,
    /// The channel's guest group (a TSG `GPFIFO_SCHEDULE` names it). Immutable.
    pub tsg: Option<u32>,
    /// Our host ring's channel, for the acts that stop, restart, evict and preempt it. Immutable.
    pub host: kf_host::Channel,
    /// The guest engine the channel was declared on. Immutable.
    pub guest_engine: u32,
    /// The host ring's GR context `(object, class)`, fixed at its birth.
    pub gr_context: Option<(u32, u32)>,
    /// The host ring's video context `(object, class, engine)`, fixed at its birth.
    pub video_context: Option<(u32, u32, u32)>,
    scheduled: AtomicBool,
    stopped: AtomicBool,
    disabled: AtomicBool,
    dead: AtomicBool,
    /// The pump epoch: ODD while a pump of this channel is running, EVEN between pumps; every pump
    /// start and end adds one. See [`SlotMeta::pump_enter`].
    pump_epoch: AtomicU64,
    /// The guest's `GPU_PROMOTE_CTX` / `GPU_EVICT_CTX` statements for this channel (act thread).
    pub ctx: Mutex<crate::chan::CtxBind>,
}

impl SlotMeta {
    /// The facts of a channel born on token `guest_idx` in group `tsg`, its host ring `host`,
    /// declared on `guest_engine`; `scheduled` as the host-owned-scheduling switch says.
    #[must_use]
    pub fn new(
        guest_idx: u32,
        tsg: Option<u32>,
        host: kf_host::Channel,
        guest_engine: u32,
        contexts: HostContexts,
        scheduled: bool,
    ) -> Self {
        SlotMeta {
            guest_idx,
            tsg,
            host,
            guest_engine,
            gr_context: contexts.0,
            video_context: contexts.1,
            scheduled: AtomicBool::new(scheduled),
            stopped: AtomicBool::new(false),
            disabled: AtomicBool::new(false),
            dead: AtomicBool::new(false),
            pump_epoch: AtomicU64::new(0),
            ctx: Mutex::new(crate::chan::CtxBind::default()),
        }
    }

    /// Whether the host ring owns a real context on `engine` (`HostRing::owns_context`, read from
    /// the birth-fixed contexts).
    #[must_use]
    pub fn owns_context(&self, engine: u32) -> bool {
        (engine == kf_abi::submit::ENGINE_TYPE_GRAPHICS && self.gr_context.is_some())
            || self.video_context.is_some_and(|(_, _, e)| e == engine)
    }

    /// Whether the channel is scheduled now.
    #[must_use]
    pub fn is_scheduled(&self) -> bool {
        self.scheduled.load(Ordering::SeqCst)
    }

    /// Whether the guest stopped the channel and has not restarted it.
    #[must_use]
    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    /// Whether the guest disabled the channel (`DISABLE_CHANNELS`) and has not re-enabled it.
    #[must_use]
    pub fn is_disabled(&self) -> bool {
        self.disabled.load(Ordering::SeqCst)
    }

    /// Whether the pump refused the channel for good (its reason is in the slot).
    #[must_use]
    pub fn is_dead(&self) -> bool {
        self.dead.load(Ordering::Acquire)
    }

    /// ★ **A pump of this channel starts** (the worker, right after it took the pump lock and BEFORE
    /// it reads the schedule flags): the epoch becomes odd until the guard drops. Together with
    /// [`SlotMeta::quiesce_mark`] this is a Dekker pair on sequentially consistent operations: of a
    /// pump starting and a disable's flags being set, either the pump SEES the flags (and submits
    /// nothing) or the disable SEES the pump (and waits for it). No pump can slip between.
    #[must_use]
    pub fn pump_enter(&self) -> PumpGuard<'_> {
        self.pump_epoch.fetch_add(1, Ordering::SeqCst);
        PumpGuard(self)
    }

    /// Whether the flags let a pump submit work now (read AFTER [`SlotMeta::pump_enter`]).
    #[must_use]
    pub fn may_pump(&self) -> bool {
        !self.is_dead() && self.is_scheduled() && !self.is_disabled() && !self.is_stopped()
    }

    /// ★ After a disable/stop/evict set its flags: the epoch of the pump still running, if one is — a
    /// pump that STARTED BEFORE the flags (it may already be past its flag check, and will still
    /// forward what it read). `None`: no pump is running, and any later one sees the flags.
    #[must_use]
    pub fn quiesce_mark(&self) -> Option<u64> {
        let e = self.pump_epoch.load(Ordering::SeqCst);
        (e & 1 == 1).then_some(e)
    }

    /// Whether the pump `mark` was taken for has ended.
    #[must_use]
    pub fn pump_ended(&self, mark: u64) -> bool {
        self.pump_epoch.load(Ordering::SeqCst) != mark
    }

    /// Set the schedule flag.
    pub fn set_scheduled(&self, on: bool) {
        self.scheduled.store(on, Ordering::SeqCst);
    }

    /// Set the stopped flag.
    pub fn set_stopped(&self, on: bool) {
        self.stopped.store(on, Ordering::SeqCst);
    }

    /// Set the disabled flag.
    pub fn set_disabled(&self, on: bool) {
        self.disabled.store(on, Ordering::SeqCst);
    }

    /// The pump gave the channel up.
    pub fn set_dead(&self) {
        self.dead.store(true, Ordering::Release);
    }
}

/// A pump in flight ([`SlotMeta::pump_enter`]); dropping it ends the pump.
#[derive(Debug)]
pub struct PumpGuard<'a>(&'a SlotMeta);

impl Drop for PumpGuard<'_> {
    fn drop(&mut self) {
        self.0.pump_epoch.fetch_add(1, Ordering::SeqCst);
    }
}

/// A channel's slot: the drainer's atomics beside the pump's lock.
#[derive(Debug)]
pub struct SlotCell<T> {
    /// The drainer's half — no lock.
    pub meta: SlotMeta,
    pump: TimedMutex<T>,
}

impl<T> SlotCell<T> {
    /// A slot whose pump state is `v`.
    pub fn new(stall: &'static Stall, meta: SlotMeta, v: T) -> Self {
        SlotCell {
            meta,
            pump: TimedMutex::new(stall, LockId::Slot, v),
        }
    }

    /// The pump state, blocking (tests: the control that shows what the drainer no longer does).
    ///
    /// # Errors
    /// Poisoned.
    #[cfg(test)]
    pub fn lock(&self) -> std::sync::LockResult<MutexGuard<'_, T>> {
        self.pump.lock()
    }

    /// The pump state, without blocking.
    ///
    /// # Errors
    /// Held by another thread, or poisoned.
    pub fn try_lock(&self) -> TryLockResult<MutexGuard<'_, T>> {
        self.pump.try_lock()
    }
}

/// What a guest `GPFIFO_SCHEDULE` on a Translated channel needs done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleStep {
    /// The channel was STOPPED and the guest enables it: the host ring must be re-enabled first, an
    /// act (host verbs), which sets `scheduled` itself.
    Restart,
    /// The flag was set; the answer is OK now. `token` is the guest token (for the log and for
    /// waking a worker when work is already queued).
    Set {
        /// The guest token.
        token: u32,
    },
}

/// ★ The drainer's half of a schedule statement: no lock, no wait.
pub fn plan_schedule(meta: &SlotMeta, enable: bool) -> ScheduleStep {
    if enable && meta.is_stopped() {
        return ScheduleStep::Restart;
    }
    meta.set_scheduled(enable);
    ScheduleStep::Set {
        token: meta.guest_idx,
    }
}

/// ★ The drainer's half of a free statement on a Translated channel: the pump is told to stop at
/// once. (The act thread waits the pump out with the lock before it frees the twin.)
pub fn stop_pump(meta: &SlotMeta) {
    meta.set_scheduled(false);
}

/// ★ Whether `meta` belongs to the guest group `tsg` — the drainer's TSG-schedule filter.
#[must_use]
pub fn in_group(meta: &SlotMeta, tsg: u32) -> bool {
    meta.tsg == Some(tsg)
}

/// Builds the refusal when a disable's wait is given up (`still_running` pumps).
pub type TimedOut<C, T> = Box<dyn FnOnce(&C, usize) -> T + Send>;

/// Slots whose in-flight pump a disable is waiting for: `(slot, mark)` from [`SlotMeta::quiesce_mark`].
pub type Marks<S> = Vec<(Arc<SlotCell<S>>, u64)>;

/// ★ **Hold a disable's reply until no pump that started before the disable is still running** —
/// as continuations on the act thread's event loop (`kf_chan::actloop`): nothing sleeps and no thread
/// blocks; a 1 ms timer resumes the check, the drainer and the act queue keep accepting other work.
/// When every marked pump has ended, `then` runs (the host verb, or the answer). After `tries` timer
/// steps the wait is given up and `timed_out(ctx, still_running)` builds the refusal — named, never a
/// silent success: the guest is not told "disabled" while the channel may still forward work.
pub fn quiesce<C: ?Sized + 'static, S: Send + Sync + 'static, T: 'static>(
    ctx: &C,
    marks: Marks<S>,
    tries: u32,
    then: Cont<C, T>,
    timed_out: TimedOut<C, T>,
) -> Step<C, T> {
    let marks: Marks<S> = marks
        .into_iter()
        .filter(|(slot, mark)| !slot.meta.pump_ended(*mark))
        .collect();
    if marks.is_empty() {
        return then(ctx);
    }
    if tries == 0 {
        return Step::Done(timed_out(ctx, marks.len()));
    }
    Step::Wait(
        Wait::Timer(std::time::Duration::from_millis(1)),
        Box::new(move |c: &C| quiesce(c, marks, tries - 1, then, timed_out)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::mpsc;
    use std::time::Duration;

    fn cell() -> Arc<SlotCell<u32>> {
        Arc::new(SlotCell::new(
            Stall::leak(),
            SlotMeta::new(
                0x2a,
                Some(0x77),
                kf_host::Channel {
                    tsg: 0x91,
                    chan: 0x92,
                    token: 0x93,
                    born_user: None,
                },
                0x1,
                (Some((1, 2)), None),
                false,
            ),
            0,
        ))
    }

    /// A worker mid-pump: holds the slot lock until told to let go. Returns the release switch.
    fn worker_in_pump(c: &Arc<SlotCell<u32>>) -> (mpsc::Sender<()>, std::thread::JoinHandle<()>) {
        let (held_tx, held_rx) = mpsc::channel();
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let c = Arc::clone(c);
        let h = std::thread::spawn(move || {
            let mut g = c.lock().unwrap();
            *g += 1;
            held_tx.send(()).unwrap();
            let _ = go_rx.recv();
        });
        held_rx.recv().unwrap();
        (go_tx, h)
    }

    /// Run `f` on a thread of its own (the drainer) and wait at most 2 s for it: a drainer path
    /// that took the pump lock would still be waiting when this gives up.
    fn drainer<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> Option<R> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(Duration::from_secs(2)).ok()
    }

    /// ★ FALSIFIER of the drainer stall (A): while a worker holds the slot lock (a pump in flight),
    /// every statement-path read and write of the slot returns at once. Before the split these were
    /// `slot.lock()` calls and each one waited for the pump.
    #[test]
    fn the_drainer_paths_return_while_a_worker_holds_the_pump_lock() {
        let c = cell();
        let (go, worker) = worker_in_pump(&c);
        let d = Arc::clone(&c);
        let got = drainer(move || {
            let token = d.meta.guest_idx; // the Token statement
            let member = in_group(&d.meta, 0x77); // the TSG schedule filter
            let other = in_group(&d.meta, 0x78);
            let on = plan_schedule(&d.meta, true); // schedule_translated
            let off = plan_schedule(&d.meta, false);
            stop_pump(&d.meta); // the free statement
            (token, member, other, on, off, d.meta.is_scheduled())
        });
        let _ = go.send(());
        worker.join().unwrap();
        let (token, member, other, on, off, scheduled) =
            got.expect("a drainer path waited for the pump lock");
        assert_eq!(token, 0x2a);
        assert!(member && !other);
        assert_eq!(on, ScheduleStep::Set { token: 0x2a });
        assert_eq!(off, ScheduleStep::Set { token: 0x2a });
        assert!(!scheduled);
    }

    /// The control: the PUMP state really is locked meanwhile, so the test above proves the drainer
    /// paths avoid a lock that exists (a vacuous pass would be a cell with no contention at all).
    #[test]
    fn the_pump_state_is_locked_while_the_worker_holds_it() {
        let c = cell();
        let (go, worker) = worker_in_pump(&c);
        assert!(c.try_lock().is_err(), "the worker holds it");
        let d = Arc::clone(&c);
        assert!(
            drainer(move || d.lock().map(|g| *g).unwrap_or(0)).is_none(),
            "a blocking lock waits for the pump (this is what the drainer no longer does)"
        );
        let _ = go.send(());
        worker.join().unwrap();
        assert_eq!(*c.lock().unwrap(), 1);
    }

    /// ★ The `try_lock` defects, fixed: a STOPPED channel is seen as stopped, and a free stops the
    /// pump, even while a worker holds the lock.
    #[test]
    fn a_stop_and_a_free_are_seen_under_contention() {
        let c = cell();
        c.meta.set_scheduled(true);
        c.meta.set_stopped(true);
        let (go, worker) = worker_in_pump(&c);
        // Before: `slot.try_lock().is_ok_and(|g| g.stopped)` was false here.
        assert_eq!(plan_schedule(&c.meta, true), ScheduleStep::Restart);
        assert!(
            c.meta.is_scheduled(),
            "a restart leaves the flag to its act"
        );
        // Before: `try_lock … g.scheduled = false` skipped the write.
        stop_pump(&c.meta);
        assert!(!c.meta.is_scheduled());
        let _ = go.send(());
        worker.join().unwrap();
        // A disable never needs the restart.
        assert_eq!(
            plan_schedule(&c.meta, false),
            ScheduleStep::Set { token: 0x2a }
        );
    }

    // ---- the disable guarantee: no work of a pump that started before it is forwarded after the
    // guest was told ----

    struct Ctx;
    type Out = String;

    type Rig = (
        kf_chan::actloop::ActQueue<Ctx, Out>,
        Arc<kf_chan::actloop::ActStats>,
        Arc<AtomicBool>,
        std::thread::JoinHandle<()>,
        Arc<std::sync::Mutex<Vec<String>>>,
    );

    fn rig() -> Rig {
        let (q, l) = kf_chan::actloop::channel::<Ctx, Out>().unwrap();
        let stats = Arc::clone(&l.stats);
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = Arc::clone(&stop);
        let h = std::thread::spawn(move || l.run(&Ctx, &s2, Stall::leak()));
        (q, stats, stop, h, Arc::default())
    }

    fn finish(log: &Arc<std::sync::Mutex<Vec<String>>>) -> kf_chan::actloop::Finish<Ctx, Out> {
        let log = Arc::clone(log);
        Box::new(move |_, t: Out, _| log.lock().unwrap().push(t))
    }

    fn until(f: impl Fn() -> bool, ms: u64) -> bool {
        let t = std::time::Instant::now();
        while t.elapsed() < Duration::from_millis(ms) {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        f()
    }

    /// ★ FALSIFIER of the regression: a worker is MID-PUMP, the guest's disable arrives. The drainer
    /// answers at once (it waits for nothing), but the disable's reply is not released until that pump
    /// has ended — and meanwhile the act thread accepts other work. (With the flag-only change the
    /// reply went out at once, and the pump went on forwarding what it had read.)
    #[test]
    fn a_disable_is_not_answered_until_the_pump_in_flight_has_ended() {
        let c = cell();
        c.meta.set_scheduled(true);
        let (go, worker) = {
            let (held_tx, held_rx) = mpsc::channel();
            let (go_tx, go_rx) = mpsc::channel::<()>();
            let c = Arc::clone(&c);
            let h = std::thread::spawn(move || {
                let _pump = c.meta.pump_enter();
                let _g = c.lock().unwrap();
                held_tx.send(()).unwrap();
                let _ = go_rx.recv();
            });
            held_rx.recv().unwrap();
            (go_tx, h)
        };
        // The drainer's half: flags at once, no wait.
        assert_eq!(
            plan_schedule(&c.meta, false),
            ScheduleStep::Set { token: 0x2a }
        );
        let mark = c.meta.quiesce_mark().expect("a pump is running");
        let (q, stats, stop, h, log) = rig();
        let marks: Marks<u32> = vec![(Arc::clone(&c), mark)];
        let first: Cont<Ctx, Out> = Box::new(move |ctx: &Ctx| {
            quiesce(
                ctx,
                marks,
                5000,
                Box::new(|_| Step::Done("disable answered".to_string())),
                Box::new(|_, n| format!("refused: {n} pump(s) still running")),
            )
        });
        assert!(q.submit("disable", first, finish(&log)));
        let second: Cont<Ctx, Out> = Box::new(|_| Step::Done("other act".to_string()));
        std::thread::sleep(Duration::from_millis(30));
        assert!(q.submit("other", second, finish(&log)));
        assert!(
            until(|| stats.accepted.load(Ordering::Relaxed) == 2, 200),
            "the act thread did not accept other work while the disable waited"
        );
        std::thread::sleep(Duration::from_millis(60));
        assert!(
            log.lock().unwrap().is_empty(),
            "the disable was answered while a pump that started before it was running: {:?}",
            log.lock().unwrap()
        );
        // The pump ends: the reply is released, then the next act runs (statement order).
        go.send(()).unwrap();
        worker.join().unwrap();
        assert!(until(|| log.lock().unwrap().len() == 2, 1000));
        assert_eq!(*log.lock().unwrap(), vec!["disable answered", "other act"]);
        stop.store(true, Ordering::Release);
        q.poke();
        h.join().unwrap();
    }

    /// A pump that never ends cannot hold the reply for ever: it is refused, named, after the
    /// deadline (never a silent success).
    #[test]
    fn a_pump_that_never_ends_gets_the_disable_refused_by_name_at_the_deadline() {
        let c = cell();
        let _pump = c.meta.pump_enter();
        c.meta.set_scheduled(false);
        let mark = c.meta.quiesce_mark().unwrap();
        let (q, _stats, stop, h, log) = rig();
        let marks: Marks<u32> = vec![(Arc::clone(&c), mark)];
        let first: Cont<Ctx, Out> = Box::new(move |ctx: &Ctx| {
            quiesce(
                ctx,
                marks,
                20,
                Box::new(|_| Step::Done("answered".to_string())),
                Box::new(|_, n| format!("refused:{n}")),
            )
        });
        assert!(q.submit("disable", first, finish(&log)));
        assert!(until(|| !log.lock().unwrap().is_empty(), 2000));
        assert_eq!(*log.lock().unwrap(), vec!["refused:1"]);
        stop.store(true, Ordering::Release);
        q.poke();
        h.join().unwrap();
    }

    /// With no pump running the disable is answered at once, and a pump that STARTS after the flags
    /// sees them and submits nothing.
    #[test]
    fn a_pump_that_starts_after_the_disable_sees_it_and_submits_nothing() {
        let c = cell();
        c.meta.set_scheduled(true);
        assert!(
            c.meta.quiesce_mark().is_none(),
            "no pump: nothing to wait for"
        );
        let _ = plan_schedule(&c.meta, false);
        let pump = c.meta.pump_enter();
        assert!(
            !c.meta.may_pump(),
            "the late pump sees `scheduled == false`"
        );
        drop(pump);
        // stop, disable and dead are seen the same way
        c.meta.set_scheduled(true);
        c.meta.set_stopped(true);
        let _p = c.meta.pump_enter();
        assert!(!c.meta.may_pump());
    }

    /// The Dekker pair under contention: a pumper (enter, check the flags, "forward" if allowed, exit)
    /// races a disabler (set the flags, mark, wait for the marked pump). After the disabler is
    /// released, the pumper has forwarded NOTHING new — over many rounds.
    #[test]
    fn no_pump_forwards_after_the_disable_is_released() {
        for round in 0..300u32 {
            let c = cell();
            c.meta.set_scheduled(true);
            let forwarded = Arc::new(std::sync::atomic::AtomicU64::new(0));
            let stop = Arc::new(AtomicBool::new(false));
            let pumper = {
                let (c, f, stop) = (Arc::clone(&c), Arc::clone(&forwarded), Arc::clone(&stop));
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Acquire) {
                        let _pump = c.meta.pump_enter();
                        if c.meta.may_pump() {
                            // forward: a little work, so the window is real
                            for _ in 0..50 {
                                std::hint::spin_loop();
                            }
                            f.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                })
            };
            std::thread::sleep(Duration::from_micros(u64::from(round % 7) * 40));
            let _ = plan_schedule(&c.meta, false);
            if let Some(mark) = c.meta.quiesce_mark() {
                while !c.meta.pump_ended(mark) {
                    std::hint::spin_loop();
                }
            }
            let at_release = forwarded.load(Ordering::SeqCst);
            std::thread::sleep(Duration::from_micros(300));
            let later = forwarded.load(Ordering::SeqCst);
            stop.store(true, Ordering::Release);
            pumper.join().unwrap();
            assert_eq!(
                later, at_release,
                "round {round}: a pump forwarded after the disable was released"
            );
        }
    }
}
