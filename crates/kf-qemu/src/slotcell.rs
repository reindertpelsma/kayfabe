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

use kf_chan::stall::{LockId, Stall, TimedMutex};
use std::sync::atomic::{AtomicBool, Ordering};
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
        self.scheduled.load(Ordering::Acquire)
    }

    /// Whether the guest stopped the channel and has not restarted it.
    #[must_use]
    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    /// Whether the guest disabled the channel (`DISABLE_CHANNELS`) and has not re-enabled it.
    #[must_use]
    pub fn is_disabled(&self) -> bool {
        self.disabled.load(Ordering::Acquire)
    }

    /// Whether the pump refused the channel for good (its reason is in the slot).
    #[must_use]
    pub fn is_dead(&self) -> bool {
        self.dead.load(Ordering::Acquire)
    }

    /// Set the schedule flag.
    pub fn set_scheduled(&self, on: bool) {
        self.scheduled.store(on, Ordering::Release);
    }

    /// Set the stopped flag.
    pub fn set_stopped(&self, on: bool) {
        self.stopped.store(on, Ordering::Release);
    }

    /// Set the disabled flag.
    pub fn set_disabled(&self, on: bool) {
        self.disabled.store(on, Ordering::Release);
    }

    /// The pump gave the channel up.
    pub fn set_dead(&self) {
        self.dead.store(true, Ordering::Release);
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
}
