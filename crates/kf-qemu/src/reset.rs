//! ★★★★★ **The device reset — one guest life ends, the next one starts on a fresh device**
//! (`docs/design/V3_DEVICE_RESET.md`).
//!
//! QEMU resets the PCI function (guest reboot, the reset button, a bugcheck or triple-fault reset,
//! QMP `system_reset`, a bus reset from the guest); the C device's Resettable `hold` phase calls
//! `kf3_device_reset`, which reaches [`Reset::request`] — and returns. Nothing in that call waits:
//! it runs on the main loop (or on a vCPU writing a bridge's config space) with the BQL held, and
//! nothing may block under a lock a vCPU takes.
//!
//! The reset itself is **tier B** of `THE_CONSTRAINTS.md` §40 (the guest driver's lifetime: GSP
//! phase, channels, VA spaces, mappings, BAR views) and leaves tier A (the store reservation, the
//! host session, the walker's context, the windows, the threads) alone. Each piece of tier-B state
//! is reset by the thread that OWNS it, at a point where it holds nothing half-done, in the order
//! §9 requires (`kf_core::lifetime::ORDER`): vCPUs out, the privileged ring and the GSP state
//! machine, every guest client freed through the channel plane's own free path (host twins, their
//! tokens, doorbell registrations, relays), then the memory plane (mirrors, BAR windows, the
//! walker's per-object state), then the register shadow republished — and only then does the
//! device accept guest writes again.
//!
//! # The gate ([`Gate`])
//!
//! Every vCPU entry that changes device state (a BAR0 write, a BAR1 usermode-view write, a BAR0
//! read exit with a side effect) passes the gate: one `fetch_add` on entry, one `fetch_sub` on
//! exit, no lock and no wait. While a reset is pending the gate is CLOSED and the entry is
//! dropped (counted) — a device in reset does not accept accesses. The drainer starts the reset
//! only once no vCPU is between entry and exit ([`Gate::quiet`]); it never waits for that, it
//! asks again on its next pass.
//!
//! # The coordinator ([`Reset::poll`])
//!
//! Driven by the register drainer on every pass while a reset is pending, and never blocking:
//! each stage either completes synchronously (bounded work on the drainer's own state) or asks
//! another thread to reset its state and is polled for that thread's acknowledgement. A reset
//! requested while one is running restarts the sequence (every stage is idempotent); the gate
//! stays closed until a sequence completes with no newer request behind it.
//!
//! ⊘ No completion is forged at any stage: work that reached the GPU is stopped by the host's own
//! free of its channel, and nothing is written to guest memory on its behalf.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// The gate's CLOSED bit; the low bits count vCPU entries in progress.
const CLOSED: u64 = 1 << 63;

/// ★ The vCPU gate. Lock-free on the vCPU: one atomic add on entry, one subtract on exit.
#[derive(Debug, Default)]
pub struct Gate {
    word: AtomicU64,
    dropped: AtomicU64,
}

/// One vCPU entry in progress; the gate counts it until this is dropped.
#[derive(Debug)]
#[must_use = "the entry is counted until this pass is dropped"]
pub struct Pass<'a>(&'a Gate);

impl Drop for Pass<'_> {
    fn drop(&mut self) {
        self.0.word.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Gate {
    /// ★ THE vCPU CHECK. `None`: the device is in reset and this access is dropped (counted).
    ///
    /// ⊘ SeqCst on both sides: a closer's `fetch_or` and an entry's `fetch_add` are totally
    /// ordered, so either the entry sees CLOSED (and leaves) or the closer's later [`Gate::quiet`]
    /// sees the entry counted. There is no third interleaving.
    #[inline]
    pub fn enter(&self) -> Option<Pass<'_>> {
        let w = self.word.fetch_add(1, Ordering::SeqCst);
        if w & CLOSED != 0 {
            self.word.fetch_sub(1, Ordering::SeqCst);
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        Some(Pass(self))
    }

    /// Close the gate (idempotent). Entries already inside finish; new ones are dropped.
    pub fn close(&self) {
        self.word.fetch_or(CLOSED, Ordering::SeqCst);
    }

    /// Open the gate.
    pub fn open(&self) {
        self.word.fetch_and(!CLOSED, Ordering::SeqCst);
    }

    /// Closed?
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.word.load(Ordering::SeqCst) & CLOSED != 0
    }

    /// No vCPU entry is in progress.
    #[must_use]
    pub fn quiet(&self) -> bool {
        self.word.load(Ordering::SeqCst) & !CLOSED == 0
    }

    /// Accesses dropped while closed (status line).
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// ★ The reset's stages, in the order they run (`docs/design/V3_DEVICE_RESET.md` §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Stage {
    /// No reset pending.
    #[default]
    Idle,
    /// The gate is closed; waiting for every vCPU entry in progress to leave.
    Quiesce,
    /// The drainer's own state: the privileged ring, the GSP state machine and its chain, the
    /// interrupt tree, the register-shadow helpers — synchronous, bounded.
    Drainer,
    /// Every guest client freed through the channel plane's free path (act thread), and the
    /// display worker's per-life state reset; waiting for both acknowledgements.
    Release,
    /// The memory plane (VA thread): mirrors retired, BAR windows back to their boot view, the
    /// walker's per-object state reset; waiting for its acknowledgement.
    Memory,
    /// Tokens swept, the register shadow restored and republished, interrupt lines drained; then
    /// the gate opens.
    Finish,
}

/// ★ The owners of tier-B state, as the coordinator drives them. The device implements it; the
/// tests implement it with fakes. Every method is called on the register drainer and must not
/// block: `begin_*` posts a request to the owning thread, `*_done` reads its acknowledgement.
pub trait Owners {
    /// The drainer's own state, reset synchronously ([`Stage::Drainer`]).
    fn reset_drainer_state(&self, epoch: u64);
    /// Ask the channel plane (act thread) and the display worker to release this life.
    fn begin_release(&self, epoch: u64);
    /// Both have acknowledged `epoch`.
    fn released(&self, epoch: u64) -> bool;
    /// Ask the VA thread to reset the memory plane.
    fn begin_memory(&self, epoch: u64);
    /// The VA thread has acknowledged `epoch`.
    fn memory_done(&self, epoch: u64) -> bool;
    /// The last, synchronous step before the gate opens ([`Stage::Finish`]).
    fn finish(&self, epoch: u64);
    /// A stage has waited `waited` for its acknowledgement (bounded logging is the caller's).
    fn stalled(&self, _stage: Stage, _epoch: u64, _waited: std::time::Duration) {}
}

/// What one [`Reset::poll`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// No reset pending.
    Idle,
    /// A reset is in progress at this stage (the drainer polls again soon).
    Pending(Stage),
    /// The reset of `epoch` completed in this poll; the gate is open.
    Completed(u64),
}

#[derive(Debug, Default)]
struct Run {
    stage: Stage,
    epoch: u64,
    since: Option<std::time::Instant>,
    stall_said: bool,
}

/// How long a stage waits before [`Owners::stalled`] is told (once per stage and epoch).
pub const STALL_AFTER: std::time::Duration = std::time::Duration::from_secs(5);

/// ★ The reset: the gate, the request counter, and the drainer's sequence.
#[derive(Debug, Default)]
pub struct Reset {
    /// The vCPU gate.
    pub gate: Gate,
    requested: AtomicU64,
    completed: AtomicU64,
    run: Mutex<Run>,
}

impl Reset {
    /// ★ The main loop's call (QEMU's Resettable hold phase): close the gate and count a request.
    /// Returns the request's number. Never waits; the caller wakes the drainer.
    pub fn request(&self) -> u64 {
        self.gate.close();
        self.requested.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// A reset is requested and not yet completed (one atomic load: the drainer's hot path).
    #[must_use]
    pub fn pending(&self) -> bool {
        self.requested.load(Ordering::Acquire) != self.completed.load(Ordering::Acquire)
    }

    /// Requests so far.
    #[must_use]
    pub fn requested(&self) -> u64 {
        self.requested.load(Ordering::Acquire)
    }

    /// Resets completed so far (each covers every request up to it).
    #[must_use]
    pub fn completed(&self) -> u64 {
        self.completed.load(Ordering::Acquire)
    }

    /// The stage the drainer is at (status line).
    #[must_use]
    pub fn stage(&self) -> Stage {
        self.run.lock().map_or(Stage::Idle, |r| r.stage)
    }

    /// ★ The drainer's step. Runs every stage that can complete now and returns where it stopped.
    /// Never blocks: a stage waiting for another thread returns [`Progress::Pending`].
    pub fn poll(&self, o: &dyn Owners) -> Progress {
        if !self.pending() {
            return Progress::Idle;
        }
        let Ok(mut r) = self.run.lock() else {
            return Progress::Pending(Stage::Idle);
        };
        loop {
            match r.stage {
                Stage::Idle => {
                    // The epoch is the newest request: everything up to it is covered by this run.
                    r.epoch = self.requested.load(Ordering::Acquire);
                    // Idempotent: [`Reset::request`] closed it; a request that raced the previous
                    // run's open may not have.
                    self.gate.close();
                    self.enter(&mut r, Stage::Quiesce);
                }
                Stage::Quiesce => {
                    if !self.gate.quiet() {
                        return self.waiting(&mut r, o);
                    }
                    self.enter(&mut r, Stage::Drainer);
                }
                Stage::Drainer => {
                    o.reset_drainer_state(r.epoch);
                    o.begin_release(r.epoch);
                    self.enter(&mut r, Stage::Release);
                }
                Stage::Release => {
                    if !o.released(r.epoch) {
                        return self.waiting(&mut r, o);
                    }
                    o.begin_memory(r.epoch);
                    self.enter(&mut r, Stage::Memory);
                }
                Stage::Memory => {
                    if !o.memory_done(r.epoch) {
                        return self.waiting(&mut r, o);
                    }
                    self.enter(&mut r, Stage::Finish);
                }
                Stage::Finish => {
                    o.finish(r.epoch);
                    let newest = self.requested.load(Ordering::Acquire);
                    if newest != r.epoch {
                        // ⊘ A reset arrived while this one ran: the device it produced may already
                        // hold the new request's predecessor's state — run the whole sequence again
                        // (the gate stays closed).
                        r.epoch = newest;
                        self.enter(&mut r, Stage::Quiesce);
                        continue;
                    }
                    let epoch = r.epoch;
                    self.enter(&mut r, Stage::Idle);
                    self.completed.store(epoch, Ordering::Release);
                    // ⊘ A request racing this open (its close before the open, its count after the
                    // load) leaves the gate open until the next poll, whose first step closes it
                    // again: the writes in between are ordered before that request, as writes on a
                    // running device are.
                    if self.requested.load(Ordering::Acquire) == epoch {
                        self.gate.open();
                    }
                    return Progress::Completed(epoch);
                }
            }
        }
    }

    fn enter(&self, r: &mut Run, s: Stage) {
        r.stage = s;
        r.since = Some(std::time::Instant::now());
        r.stall_said = false;
    }

    fn waiting(&self, r: &mut Run, o: &dyn Owners) -> Progress {
        if let Some(t) = r.since
            && !r.stall_said
            && t.elapsed() >= STALL_AFTER
        {
            r.stall_said = true;
            o.stalled(r.stage, r.epoch, t.elapsed());
        }
        Progress::Pending(r.stage)
    }

    /// The status line's fragment.
    #[must_use]
    pub fn status(&self) -> String {
        format!(
            "reset[req={} done={} stage={:?} dropped={}]",
            self.requested(),
            self.completed(),
            self.stage(),
            self.gate.dropped()
        )
    }
}

/// ★ An acknowledgement word: the owning thread stores the epoch it finished; the drainer compares.
/// Requests are an epoch too, so a late acknowledgement of an older request never satisfies a
/// newer one.
#[derive(Debug, Default)]
pub struct Ack {
    want: AtomicU64,
    done: AtomicU64,
}

impl Ack {
    /// The drainer asks for `epoch`.
    pub fn request(&self, epoch: u64) {
        self.want.fetch_max(epoch, Ordering::AcqRel);
    }

    /// The owner: the epoch it owes, if any (`want > done`).
    #[must_use]
    pub fn owed(&self) -> Option<u64> {
        let w = self.want.load(Ordering::Acquire);
        (w > self.done.load(Ordering::Acquire)).then_some(w)
    }

    /// The owner finished `epoch`.
    pub fn done(&self, epoch: u64) {
        self.done.fetch_max(epoch, Ordering::AcqRel);
    }

    /// The drainer: has `epoch` been acknowledged?
    #[must_use]
    pub fn is_done(&self, epoch: u64) -> bool {
        self.done.load(Ordering::Acquire) >= epoch
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn an_open_gate_admits_and_counts_the_entry_until_it_leaves() {
        let g = Gate::default();
        assert!(g.quiet() && !g.is_closed());
        let p = g.enter().expect("open");
        assert!(!g.quiet());
        drop(p);
        assert!(g.quiet());
        assert_eq!(g.dropped(), 0);
    }

    #[test]
    fn a_closed_gate_drops_the_entry_and_counts_it() {
        let g = Gate::default();
        g.close();
        assert!(g.enter().is_none());
        assert!(g.quiet(), "a dropped entry leaves no count behind");
        assert_eq!(g.dropped(), 1);
        g.open();
        assert!(g.enter().is_some());
    }

    #[test]
    fn closing_does_not_wait_for_an_entry_in_progress_and_quiet_says_when_it_left() {
        let g = Gate::default();
        let p = g.enter().expect("open");
        g.close();
        assert!(!g.quiet(), "the entry that started before the close is still inside");
        drop(p);
        assert!(g.quiet());
    }

    /// ★ The race the gate exists for: vCPU threads entering and leaving while the main loop
    /// closes. Whenever the drainer sees `quiet()` after the close, no thread may be inside — every
    /// thread marks itself inside while it holds a pass.
    #[test]
    fn once_closed_and_quiet_no_vcpu_is_inside_under_contention() {
        for _ in 0..20 {
            let g = Arc::new(Gate::default());
            let inside = Arc::new(AtomicU64::new(0));
            let stop = Arc::new(AtomicBool::new(false));
            let vcpus: Vec<_> = (0..4)
                .map(|_| {
                    let (g, inside, stop) = (g.clone(), inside.clone(), stop.clone());
                    std::thread::spawn(move || {
                        let mut admitted = 0u64;
                        while !stop.load(Ordering::Relaxed) {
                            if let Some(p) = g.enter() {
                                inside.fetch_add(1, Ordering::SeqCst);
                                std::hint::spin_loop();
                                inside.fetch_sub(1, Ordering::SeqCst);
                                drop(p);
                                admitted += 1;
                            }
                        }
                        admitted
                    })
                })
                .collect();
            std::thread::sleep(std::time::Duration::from_millis(2));
            g.close();
            let mut spins = 0u64;
            while !g.quiet() {
                spins += 1;
                std::hint::spin_loop();
            }
            // ⊘ `quiet()` itself may flicker here: a refused entry counts for the nanoseconds between
            // its add and its subtract. What may never happen is a thread INSIDE.
            for _ in 0..10_000 {
                assert_eq!(inside.load(Ordering::SeqCst), 0, "a vCPU inside a closed, quiet gate");
            }
            stop.store(true, Ordering::Relaxed);
            for v in vcpus {
                v.join().expect("vcpu");
            }
            assert!(g.dropped() > 0 || spins == 0);
        }
    }

    /// A fake set of owners: records the order of calls, acknowledges after `delay` polls, and
    /// keeps a ledger of "host resources" a life acquires and the release gives back.
    #[derive(Default)]
    struct Fake {
        calls: RefCell<Vec<String>>,
        delay: Cell<u32>,
        release_polls: Cell<u32>,
        memory_polls: Cell<u32>,
        release_asked: Cell<u64>,
        memory_asked: Cell<u64>,
        /// Live "host resources" (twins, mappings, registrations) of the current life.
        live: Cell<u64>,
        /// Ever acquired / ever released.
        acquired: Cell<u64>,
        released_total: Cell<u64>,
        /// The "device state" a fresh device has (0) and a used one does not.
        dirty: Cell<u64>,
        /// A callback run inside `begin_release` (a request arriving mid-run).
        during_release: RefCell<Option<Box<dyn Fn()>>>,
        stalls: Cell<u32>,
    }

    impl Fake {
        fn note(&self, s: impl Into<String>) {
            self.calls.borrow_mut().push(s.into());
        }
        fn live_a_life(&self, n: u64) {
            self.live.set(self.live.get() + n);
            self.acquired.set(self.acquired.get() + n);
            self.dirty.set(self.dirty.get() + 1);
        }
    }

    impl Owners for Fake {
        fn reset_drainer_state(&self, e: u64) {
            self.note(format!("drainer {e}"));
            self.dirty.set(0);
        }
        fn begin_release(&self, e: u64) {
            self.note(format!("release {e}"));
            self.release_asked.set(e);
            self.release_polls.set(self.delay.get());
            if let Some(f) = self.during_release.borrow_mut().take() {
                f();
            }
        }
        fn released(&self, e: u64) -> bool {
            assert_eq!(self.release_asked.get(), e);
            if self.release_polls.get() > 0 {
                self.release_polls.set(self.release_polls.get() - 1);
                return false;
            }
            self.released_total
                .set(self.released_total.get() + self.live.get());
            self.live.set(0);
            true
        }
        fn begin_memory(&self, e: u64) {
            self.note(format!("memory {e}"));
            assert_eq!(self.live.get(), 0, "memory reset only after the channels are gone");
            self.memory_asked.set(e);
            self.memory_polls.set(self.delay.get());
        }
        fn memory_done(&self, e: u64) -> bool {
            assert_eq!(self.memory_asked.get(), e);
            if self.memory_polls.get() > 0 {
                self.memory_polls.set(self.memory_polls.get() - 1);
                return false;
            }
            true
        }
        fn finish(&self, e: u64) {
            self.note(format!("finish {e}"));
        }
        fn stalled(&self, _s: Stage, _e: u64, _w: std::time::Duration) {
            self.stalls.set(self.stalls.get() + 1);
        }
    }

    fn run_to_completion(r: &Reset, o: &Fake) -> Vec<Progress> {
        let mut seen = Vec::new();
        for _ in 0..1000 {
            let p = r.poll(o);
            seen.push(p);
            if matches!(p, Progress::Completed(_) | Progress::Idle) {
                return seen;
            }
        }
        panic!("the reset never completed: {seen:?}");
    }

    #[test]
    fn with_no_request_the_drainer_does_nothing() {
        let r = Reset::default();
        let o = Fake::default();
        assert_eq!(r.poll(&o), Progress::Idle);
        assert!(o.calls.borrow().is_empty());
        assert!(!r.gate.is_closed());
    }

    #[test]
    fn a_reset_runs_every_stage_in_order_and_opens_the_gate_only_at_the_end() {
        let r = Reset::default();
        let o = Fake::default();
        o.delay.set(3);
        o.live_a_life(7);
        assert_eq!(r.request(), 1);
        assert!(r.gate.is_closed() && r.pending());
        let mut stages = Vec::new();
        loop {
            let p = r.poll(&o);
            if let Progress::Pending(s) = p {
                assert!(r.gate.is_closed(), "the gate opened before the reset finished");
                stages.push(s);
                continue;
            }
            assert_eq!(p, Progress::Completed(1));
            break;
        }
        assert!(!r.gate.is_closed() && !r.pending());
        assert_eq!(
            *o.calls.borrow(),
            ["drainer 1", "release 1", "memory 1", "finish 1"]
        );
        assert!(stages.windows(2).all(|w| w[0] <= w[1]), "stages never go back: {stages:?}");
        assert_eq!((o.live.get(), o.dirty.get()), (0, 0));
    }

    #[test]
    fn a_vcpu_inside_holds_the_reset_at_quiesce_and_nothing_is_touched() {
        let r = Reset::default();
        let o = Fake::default();
        let p = r.gate.enter().expect("open");
        r.request();
        for _ in 0..5 {
            assert_eq!(r.poll(&o), Progress::Pending(Stage::Quiesce));
        }
        assert!(o.calls.borrow().is_empty(), "no state may be reset while a vCPU is inside");
        drop(p);
        assert_eq!(run_to_completion(&r, &o).last(), Some(&Progress::Completed(1)));
    }

    #[test]
    fn a_reset_immediately_after_a_reset_runs_again_and_covers_both() {
        let r = Reset::default();
        let o = Fake::default();
        r.request();
        r.request();
        assert_eq!(run_to_completion(&r, &o).last(), Some(&Progress::Completed(2)));
        // Two requests before the first poll are ONE run: the epoch is the newest request.
        assert_eq!(o.calls.borrow().len(), 4);
        r.request();
        assert_eq!(run_to_completion(&r, &o).last(), Some(&Progress::Completed(3)));
        assert_eq!(o.calls.borrow().len(), 8);
        assert!(!r.gate.is_closed());
    }

    /// ★ A request that arrives while the sequence runs restarts it from the gate — at every stage.
    #[test]
    fn a_request_during_any_stage_restarts_the_whole_sequence_with_the_gate_closed() {
        for stop_at in [Stage::Quiesce, Stage::Release, Stage::Memory] {
            let r = Arc::new(Reset::default());
            let o = Fake::default();
            o.delay.set(2);
            if stop_at == Stage::Quiesce {
                let p = r.gate.enter().expect("open");
                r.request();
                assert_eq!(r.poll(&o), Progress::Pending(Stage::Quiesce));
                r.request();
                drop(p);
            } else {
                r.request();
                loop {
                    match r.poll(&o) {
                        Progress::Pending(s) if s == stop_at => break,
                        Progress::Pending(_) => {}
                        p => panic!("{p:?}"),
                    }
                }
                r.request();
            }
            assert!(r.gate.is_closed());
            let end = run_to_completion(&r, &o);
            assert_eq!(end.last(), Some(&Progress::Completed(2)), "{stop_at:?}");
            let calls = o.calls.borrow();
            assert_eq!(calls.last().map(String::as_str), Some("finish 2"), "{stop_at:?}: {calls:?}");
            assert!(calls.iter().any(|c| c == "drainer 2"), "{stop_at:?}: {calls:?}");
            assert!(!r.gate.is_closed());
        }
    }

    #[test]
    fn a_request_made_inside_a_stage_is_not_lost() {
        let r = Arc::new(Reset::default());
        let o = Fake::default();
        let r2 = r.clone();
        *o.during_release.borrow_mut() = Some(Box::new(move || {
            r2.request();
        }));
        r.request();
        let end = run_to_completion(&r, &o);
        assert_eq!(end.last(), Some(&Progress::Completed(2)));
        assert!(!r.pending() && !r.gate.is_closed());
    }

    #[test]
    fn a_stage_that_never_acknowledges_keeps_the_gate_closed_and_is_said_once() {
        let r = Reset::default();
        let o = Fake::default();
        o.delay.set(u32::MAX);
        r.request();
        for _ in 0..100 {
            assert_eq!(r.poll(&o), Progress::Pending(Stage::Release));
        }
        assert!(r.gate.is_closed(), "fail closed: a device that cannot reset accepts nothing");
        // Backdate the stage so the stall is reported, once.
        r.run.lock().expect("run").since =
            Some(std::time::Instant::now() - STALL_AFTER - std::time::Duration::from_millis(1));
        for _ in 0..10 {
            let _ = r.poll(&o);
        }
        assert_eq!(o.stalls.get(), 1);
    }

    /// ★ The accounting property, on the fake: N lives, each acquiring resources and ending in a
    /// reset, leave nothing live and released == acquired.
    #[test]
    fn n_reset_cycles_release_everything_every_life_acquired() {
        let r = Reset::default();
        let o = Fake::default();
        o.delay.set(1);
        for life in 1..=50u64 {
            o.live_a_life(life);
            r.request();
            assert_eq!(run_to_completion(&r, &o).last(), Some(&Progress::Completed(life)));
            assert_eq!(o.live.get(), 0);
            assert_eq!(o.acquired.get(), o.released_total.get());
            assert_eq!(o.dirty.get(), 0);
        }
        assert_eq!(r.completed(), 50);
    }

    #[test]
    fn an_ack_is_per_epoch_and_a_late_one_never_satisfies_a_newer_request() {
        let a = Ack::default();
        assert_eq!(a.owed(), None);
        a.request(1);
        assert_eq!(a.owed(), Some(1));
        a.request(2);
        assert_eq!(a.owed(), Some(2));
        a.done(1);
        assert!(a.is_done(1) && !a.is_done(2));
        assert_eq!(a.owed(), Some(2));
        a.done(2);
        assert_eq!(a.owed(), None);
        assert!(a.is_done(2));
    }
}
