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
    ///
    /// ★ 2026-10-14 (`docs/design/V3_S3_RESUME.md`): `wake` — every request this run covers was
    /// the platform's S3 WAKE (QEMU's `RESET_TYPE_WAKEUP`). Returns [`Life::Kept`] when the owner
    /// kept the guest life — the GSP-RM state a PM-transition suspend saved — and reset only what
    /// a power cycle resets on silicon (registers, interrupts); the coordinator then skips
    /// [`Stage::Release`] and [`Stage::Memory`]. Anything else is [`Life::Ended`]: the full reset.
    fn reset_drainer_state(&self, epoch: u64, wake: bool) -> Life;
    /// Ask the channel plane (act thread) and the display worker to release this life.
    fn begin_release(&self, epoch: u64);
    /// Both have acknowledged `epoch`.
    fn released(&self, epoch: u64) -> bool;
    /// Ask the VA thread to reset the memory plane. `scrub`: a NEW OS follows and the store holds
    /// the old one's bytes — the memory stage also zeroes the guest-allocatable store ([`crate::scrub`]).
    fn begin_memory(&self, epoch: u64, scrub: bool);
    /// The VA thread has acknowledged `epoch`.
    fn memory_done(&self, epoch: u64) -> bool;
    /// The last, synchronous step before the gate opens ([`Stage::Finish`]). `false`: not yet (a
    /// worker still owns a token) — nothing was changed, the drainer asks again on its next pass.
    /// `life`: what [`Owners::reset_drainer_state`] answered — a kept life keeps its tokens.
    fn finish(&self, epoch: u64, life: Life) -> bool;
    /// A stage has waited `waited` for its acknowledgement (bounded logging is the caller's).
    fn stalled(&self, _stage: Stage, _epoch: u64, _waited: std::time::Duration) {}
}

/// ★ 2026-10-14 — what a reset run did to the guest life ([`Owners::reset_drainer_state`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Life {
    /// The life ended: every stage ran, the device is as realize left it.
    #[default]
    Ended,
    /// ★ An S3 wake of a PM-suspended life: registers and interrupts reset, the guest's GSP-RM
    /// state (queues, objects, channels, address spaces, display) kept for its resume.
    Kept,
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
    life: Life,
    /// ★ 2026-10-18: this run's Memory stage was asked to scrub (read at [`Stage::Memory`]'s end to
    /// clear the owed scrub only once it completed).
    scrub: bool,
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
    /// The stage, mirrored lock-free for [`Reset::stage`] / [`Reset::status`] — callable from
    /// anywhere, [`Owners::stalled`] included (which runs while `run` is held).
    stage_now: std::sync::atomic::AtomicU8,
    /// ★ A guest access was admitted since the last reset began ([`Reset::touched`]). A reset that
    /// finds none — the machine's own reset at start-up, a reset straight after a reset — has
    /// nothing of a guest life to undo and completes at once, without waiting for any owner.
    touched: std::sync::atomic::AtomicBool,
    /// Resets that found the device pristine (status line).
    pristine: AtomicU64,
    /// Requests by where QEMU asked from: its main loop (a system reset — the vCPUs are paused —
    /// or the machine's own reset at creation) or a vCPU (a bus reset through a bridge's config
    /// write, the other vCPUs running). Status line.
    from_main: AtomicU64,
    from_vcpu: AtomicU64,
    /// Bounded waits ([`Reset::wait_completed`]) that hit their bound.
    waits_timed_out: AtomicU64,
    /// ★ 2026-10-14: a request that was NOT the platform's S3 wake arrived since the last run's
    /// Drainer stage read it — such a run never keeps a life. Set BEFORE the request is counted.
    cold_pending: std::sync::atomic::AtomicBool,
    /// ★ 2026-10-18 (review B1): a scrub is OWED — a request that a NEW OS follows (a system reset
    /// from the main loop: [`crate::scrub::decide`]) arrived and no completed Memory stage has
    /// scrubbed since. Set BEFORE the request is counted; cleared only by a Memory stage that
    /// completed WITH the scrub, or by a pristine completion over a clean store (the start-up reset).
    new_os_pending: std::sync::atomic::AtomicBool,
    /// ★ 2026-10-18 (review B1): the store may hold a guest life's bytes — set when a guest access is
    /// admitted ([`Reset::touched`]), cleared ONLY when a Memory stage with the scrub completed.
    /// Unlike `touched` it is not consumed by a reset that scrubbed nothing.
    store_dirty: std::sync::atomic::AtomicBool,
    /// Wake requests (status line) and runs that kept a PM-suspended life.
    from_wake: AtomicU64,
    kept: AtomicU64,
}

/// ★ How long the main loop waits, with every vCPU paused, for a reset to complete before it lets
/// QEMU resume them (`docs/design/V3_DEVICE_RESET.md` §3): long enough for the act thread to free a
/// guest's clients (measured: one round, milliseconds), short enough that a host that never lets go
/// cannot hold the VM — past it the reset completes asynchronously with the gate closed.
pub const WAIT_BOUND: std::time::Duration = std::time::Duration::from_secs(2);

/// What a bounded wait for a reset found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waited {
    /// The reset (and every request before it) completed: the device is as realize left it.
    Done,
    /// The bound passed with the sequence at this stage; it goes on asynchronously, gate closed.
    TimedOut(Stage),
}

impl Reset {
    /// ★ 2026-10-18: is a scrub owed — a request meant a NEW OS (a system reset from the main loop,
    /// [`crate::scrub::decide`]) and no memory stage has scrubbed since? Not consuming.
    #[must_use]
    pub fn scrub_owed(&self) -> bool {
        self.new_os_pending.load(Ordering::SeqCst)
    }

    /// ★ The main loop's call (QEMU's Resettable hold phase): close the gate and count a request.
    /// Returns the request's number. Never waits; the caller wakes the drainer.
    pub fn request(&self) -> u64 {
        self.gate.close();
        self.requested.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// [`Reset::request`], counting where QEMU asked from (`from_vcpu`: a vCPU thread — a bus
    /// reset; otherwise its main loop).
    pub fn request_from(&self, from_vcpu: bool) -> u64 {
        self.request_typed(from_vcpu, false)
    }

    /// ★ 2026-10-14: [`Reset::request_from`] with QEMU's reset type: `wake` is
    /// `RESET_TYPE_WAKEUP` (`hw/i386/pc.c:1723-1727`, the S3 resume's `qemu_devices_reset`).
    /// Any other request marks the run cold BEFORE it is counted, so the run that covers it
    /// never keeps a life ([`Owners::reset_drainer_state`]).
    pub fn request_typed(&self, from_vcpu: bool, wake: bool) -> u64 {
        if crate::scrub::decide(crate::scrub::event_of_reset(from_vcpu, wake)).is_some() {
            // a NEW OS follows: set BEFORE the request is counted, like the cold mark
            self.new_os_pending.store(true, Ordering::SeqCst);
        }
        let c = if from_vcpu { &self.from_vcpu } else { &self.from_main };
        c.fetch_add(1, Ordering::Relaxed);
        if wake {
            self.from_wake.fetch_add(1, Ordering::Relaxed);
        } else {
            self.cold_pending.store(true, Ordering::SeqCst);
        }
        self.request()
    }

    /// ★★ **The main loop's bounded wait** (QEMU's Resettable `exit` phase, every vCPU paused by
    /// `pause_all_vcpus` around `qemu_system_reset`, `system/runstate.c:868-870`): until reset
    /// `epoch` — and every request before it — completed, or `bound` passed. Never called on a
    /// vCPU, the drainer or any thread a reset stage runs on; the threads it waits for take no
    /// QEMU lock (§3 of the design note says why each one cannot need the BQL).
    pub fn wait_completed(&self, epoch: u64, bound: std::time::Duration) -> Waited {
        let t0 = std::time::Instant::now();
        loop {
            if self.completed() >= epoch {
                return Waited::Done;
            }
            if t0.elapsed() >= bound {
                self.waits_timed_out.fetch_add(1, Ordering::Relaxed);
                return Waited::TimedOut(self.stage());
            }
            std::thread::sleep(std::time::Duration::from_micros(200));
        }
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

    /// The stage the drainer is at (status line). Lock-free.
    #[must_use]
    pub fn stage(&self) -> Stage {
        match self.stage_now.load(Ordering::Acquire) {
            1 => Stage::Quiesce,
            2 => Stage::Drainer,
            3 => Stage::Release,
            4 => Stage::Memory,
            5 => Stage::Finish,
            _ => Stage::Idle,
        }
    }

    /// ★ A guest access was admitted (the vCPU path calls this after [`Gate::enter`] succeeded):
    /// the device is no longer as realize — or the last reset — left it. One relaxed load when set.
    #[inline]
    pub fn touched(&self) {
        if !self.touched.load(Ordering::Relaxed) {
            self.touched.store(true, Ordering::SeqCst);
            self.store_dirty.store(true, Ordering::SeqCst);
        }
    }

    /// ★ The drainer's step. Runs every stage that can complete now and returns where it stopped.
    /// Never blocks: a stage waiting for another thread returns [`Progress::Pending`].
    pub fn poll(&self, o: &dyn Owners) -> Progress {
        if !self.pending() {
            return Progress::Idle;
        }
        // ⊘ A poisoned sequence is still this drainer's: recover it rather than stall forever.
        let mut r = self
            .run
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            match r.stage {
                Stage::Idle => {
                    // The epoch is the newest request: everything up to it is covered by this run.
                    r.epoch = self.requested.load(Ordering::Acquire);
                    r.life = Life::Ended;
                    // Idempotent: [`Reset::request`] closed it; a request that raced the previous
                    // run's open may not have.
                    self.gate.close();
                    self.enter(&mut r, Stage::Quiesce);
                }
                Stage::Quiesce => {
                    if !self.gate.quiet() {
                        return self.waiting(&mut r, o);
                    }
                    // ⊘ Read only now: every entry admitted before the close has left, so its mark
                    // is visible. Nothing admitted since the last reset began ⇒ nothing to undo.
                    //
                    // ★ 2026-10-18 (review B1): not pristine while a scrub is owed over a dirty
                    // store — a reboot that arrives while a guest-issued reset runs finds `touched`
                    // consumed, and must still get its Memory stage.
                    let was_touched = self.touched.swap(false, Ordering::SeqCst);
                    let scrub_due = self.new_os_pending.load(Ordering::SeqCst)
                        && self.store_dirty.load(Ordering::SeqCst);
                    if !was_touched && !scrub_due {
                        self.pristine.fetch_add(1, Ordering::Relaxed);
                        // nothing of a guest life exists: a start-up request's mark dies here
                        if !self.store_dirty.load(Ordering::SeqCst) {
                            self.new_os_pending.store(false, Ordering::SeqCst);
                        }
                        // ★ 2026-10-14: the cold mark is this run's even when nothing was undone —
                        // left set, it would make the NEXT life's S3 wake a full reset.
                        self.cold_pending.store(false, Ordering::SeqCst);
                        self.enter(&mut r, Stage::Finish);
                        if let Some(p) = self.complete(&mut r) {
                            return p;
                        }
                        continue;
                    }
                    self.enter(&mut r, Stage::Drainer);
                }
                Stage::Drainer => {
                    // ★ 2026-10-14: a cold request counted after this read restarts the run at
                    // `complete` (its count is newer than the epoch), and the rerun is not
                    // pristine after a kept life (`touched` is set below), so it resets fully.
                    let wake = !self.cold_pending.swap(false, Ordering::SeqCst);
                    r.life = o.reset_drainer_state(r.epoch, wake);
                    if r.life == Life::Kept {
                        self.kept.fetch_add(1, Ordering::Relaxed);
                        // the device is not as realize left it: a later reset must not find it
                        // pristine
                        self.touched.store(true, Ordering::SeqCst);
                        self.enter(&mut r, Stage::Finish);
                    } else {
                        o.begin_release(r.epoch);
                        self.enter(&mut r, Stage::Release);
                    }
                }
                Stage::Release => {
                    if !o.released(r.epoch) {
                        return self.waiting(&mut r, o);
                    }
                    // ★ the scrub is asked only for a reboot over a store a guest dirtied
                    r.scrub = self.new_os_pending.load(Ordering::SeqCst)
                        && self.store_dirty.load(Ordering::SeqCst);
                    o.begin_memory(r.epoch, r.scrub);
                    self.enter(&mut r, Stage::Memory);
                }
                Stage::Memory => {
                    if !o.memory_done(r.epoch) {
                        return self.waiting(&mut r, o);
                    }
                    // the VA thread acknowledges only a COMPLETED scrub (a refused one keeps the
                    // stage waiting): the owed scrub is paid
                    if r.scrub {
                        self.store_dirty.store(false, Ordering::SeqCst);
                        self.new_os_pending.store(false, Ordering::SeqCst);
                    }
                    self.enter(&mut r, Stage::Finish);
                }
                Stage::Finish => {
                    if !o.finish(r.epoch, r.life) {
                        return self.waiting(&mut r, o);
                    }
                    if let Some(p) = self.complete(&mut r) {
                        return p;
                    }
                }
            }
        }
    }

    /// The end of a run: restart for a newer request (`None`), or complete and open the gate.
    fn complete(&self, r: &mut Run) -> Option<Progress> {
        let newest = self.requested.load(Ordering::Acquire);
        if newest != r.epoch {
            // ⊘ A reset arrived while this one ran: run the whole sequence again for it (the gate
            // stays closed). Nothing was admitted since, so unless the guest got in before the
            // close, the rerun finds the device pristine and completes at once.
            r.epoch = newest;
            r.life = Life::Ended;
            self.enter(r, Stage::Quiesce);
            return None;
        }
        let epoch = r.epoch;
        self.enter(r, Stage::Idle);
        self.completed.store(epoch, Ordering::Release);
        // ⊘ A request racing this open (its close before the open, its count after the load) leaves
        // the gate open until the next poll, whose first step closes it again: the writes in between
        // are ordered before that request, as writes on a running device are.
        if self.requested.load(Ordering::Acquire) == epoch {
            self.gate.open();
        }
        Some(Progress::Completed(epoch))
    }

    fn enter(&self, r: &mut Run, s: Stage) {
        r.stage = s;
        r.since = Some(std::time::Instant::now());
        r.stall_said = false;
        let n = match s {
            Stage::Idle => 0,
            Stage::Quiesce => 1,
            Stage::Drainer => 2,
            Stage::Release => 3,
            Stage::Memory => 4,
            Stage::Finish => 5,
        };
        self.stage_now.store(n, Ordering::Release);
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
            "reset[req={} (main={} vcpu={} wake={}) done={} kept={} pristine={} waits_timed_out={} stage={:?} dropped={}]",
            self.requested(),
            self.from_main.load(Ordering::Relaxed),
            self.from_vcpu.load(Ordering::Relaxed),
            self.from_wake.load(Ordering::Relaxed),
            self.completed(),
            self.kept.load(Ordering::Relaxed),
            self.pristine.load(Ordering::Relaxed),
            self.waits_timed_out.load(Ordering::Relaxed),
            self.stage(),
            self.gate.dropped()
        )
    }
}

/// ★ Whether reset number `n` (or the n-th life end) is logged: the first 16, then each power of
/// two. ⊘ A guest can reset itself in a loop; the drainer may not be made to write a line per reset
/// (owner: input-serving threads are quiet by default; a full stderr pipe would block them).
#[must_use]
pub fn loud(n: u64) -> bool {
    n <= 16 || n.is_power_of_two()
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
        assert!(
            !g.quiet(),
            "the entry that started before the close is still inside"
        );
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
                assert_eq!(
                    inside.load(Ordering::SeqCst),
                    0,
                    "a vCPU inside a closed, quiet gate"
                );
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
        /// Polls `finish` answers "not yet" (a worker still owns a token).
        finish_polls: Cell<u32>,
        /// The reset whose status a stall report reads (the device's `stalled` does).
        status_of: RefCell<Option<Arc<Reset>>>,
        status_seen: RefCell<Option<String>>,
        /// ★ 2026-10-14: the life ended with a PM-transition suspend (the GSP `Halted`, saved).
        pm_suspended: Cell<bool>,
        /// ★ 2026-10-18: the `scrub` argument of every `begin_memory`, as the real device gets it.
        scrubs: RefCell<Vec<bool>>,
        /// A callback run inside `begin_memory` (a request arriving during the memory stage).
        during_memory: RefCell<Option<Box<dyn Fn()>>>,
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
        fn reset_drainer_state(&self, e: u64, wake: bool) -> Life {
            if wake && self.pm_suspended.get() {
                self.note(format!("drainer-kept {e}"));
                self.pm_suspended.set(false);
                return Life::Kept;
            }
            self.note(format!("drainer {e}"));
            self.dirty.set(0);
            self.pm_suspended.set(false);
            Life::Ended
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
        fn begin_memory(&self, e: u64, scrub: bool) {
            self.note(format!("memory {e}"));
            self.scrubs.borrow_mut().push(scrub);
            if let Some(f) = self.during_memory.borrow_mut().take() {
                f();
            }
            assert_eq!(
                self.live.get(),
                0,
                "memory reset only after the channels are gone"
            );
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
        fn finish(&self, e: u64, life: Life) -> bool {
            if self.finish_polls.get() > 0 {
                self.finish_polls.set(self.finish_polls.get() - 1);
                return false;
            }
            self.note(match life {
                Life::Ended => format!("finish {e}"),
                Life::Kept => format!("finish-kept {e}"),
            });
            true
        }
        fn stalled(&self, _s: Stage, _e: u64, _w: std::time::Duration) {
            self.stalls.set(self.stalls.get() + 1);
            if let Some(r) = self.status_of.borrow().as_ref() {
                *self.status_seen.borrow_mut() = Some(r.status());
            }
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
        r.touched();
        assert_eq!(r.request(), 1);
        assert!(r.gate.is_closed() && r.pending());
        let mut stages = Vec::new();
        loop {
            let p = r.poll(&o);
            if let Progress::Pending(s) = p {
                assert!(
                    r.gate.is_closed(),
                    "the gate opened before the reset finished"
                );
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
        assert!(
            stages.windows(2).all(|w| w[0] <= w[1]),
            "stages never go back: {stages:?}"
        );
        assert_eq!((o.live.get(), o.dirty.get()), (0, 0));
    }

    #[test]
    fn a_vcpu_inside_holds_the_reset_at_quiesce_and_nothing_is_touched() {
        let r = Reset::default();
        let o = Fake::default();
        let p = r.gate.enter().expect("open");
        r.touched();
        r.request();
        for _ in 0..5 {
            assert_eq!(r.poll(&o), Progress::Pending(Stage::Quiesce));
        }
        assert!(
            o.calls.borrow().is_empty(),
            "no state may be reset while a vCPU is inside"
        );
        drop(p);
        assert_eq!(
            run_to_completion(&r, &o).last(),
            Some(&Progress::Completed(1))
        );
    }

    #[test]
    fn a_reset_immediately_after_a_reset_runs_again_and_covers_both() {
        let r = Reset::default();
        let o = Fake::default();
        r.touched();
        r.request();
        r.request();
        assert_eq!(
            run_to_completion(&r, &o).last(),
            Some(&Progress::Completed(2))
        );
        // Two requests before the first poll are ONE run: the epoch is the newest request.
        assert_eq!(o.calls.borrow().len(), 4);
        // A reset straight after it, with no guest access between: nothing to undo, no owner asked.
        r.request();
        assert_eq!(
            run_to_completion(&r, &o).last(),
            Some(&Progress::Completed(3))
        );
        assert_eq!(o.calls.borrow().len(), 4);
        // With a guest access between, the whole sequence again.
        r.touched();
        r.request();
        assert_eq!(
            run_to_completion(&r, &o).last(),
            Some(&Progress::Completed(4))
        );
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
            r.touched();
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
            for p in &end[..end.len() - 1] {
                assert!(matches!(p, Progress::Pending(_)), "{stop_at:?}: {end:?}");
            }
            let calls = o.calls.borrow();
            // One full run, whose epoch is the request it started for (1, or 2 when both arrived
            // before the drainer could start); the rerun for request 2 finds the device pristine
            // (the gate was closed all along) and asks no owner.
            assert_eq!(calls.len(), 4, "{stop_at:?}: {calls:?}");
            assert!(calls[3].starts_with("finish"), "{stop_at:?}: {calls:?}");
            assert!(!r.gate.is_closed() && !r.pending());
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
        r.touched();
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
        r.touched();
        r.request();
        for _ in 0..100 {
            assert_eq!(r.poll(&o), Progress::Pending(Stage::Release));
        }
        assert!(
            r.gate.is_closed(),
            "fail closed: a device that cannot reset accepts nothing"
        );
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
            r.touched();
            r.request();
            assert_eq!(
                run_to_completion(&r, &o).last(),
                Some(&Progress::Completed(life))
            );
            assert_eq!(o.live.get(), 0);
            assert_eq!(o.acquired.get(), o.released_total.get());
            assert_eq!(o.dirty.get(), 0);
        }
        assert_eq!(r.completed(), 50);
    }

    /// ★ 2026-10-18 (review B1): only a system reset from the main loop owes a scrub; a vCPU's bus
    /// reset and the S3 wake do not (the table: [`crate::scrub::decide`]).
    #[test]
    fn only_a_system_reset_from_the_main_loop_owes_a_scrub() {
        let r = Reset::default();
        r.request_typed(true, false); // a bus reset on a vCPU thread
        assert!(!r.scrub_owed(), "the guest's own bus reset keeps the OS");
        r.request_typed(false, true); // the S3 wake
        assert!(!r.scrub_owed(), "an S3 wake resumes the same OS");
        r.request_typed(false, false); // QEMU's system reset: reboot, reset button, triple fault
        assert!(r.scrub_owed(), "a system reset is a new OS");
        assert!(r.scrub_owed(), "not consuming");
    }

    /// ★ Review B1(a), the reviewer's probe: the machine's own reset at creation comes from the main
    /// loop with wake=false. It completes PRISTINE (nothing of a guest life exists), and the mark
    /// it set must not survive that completion — else the first later reset that reaches a Memory
    /// stage (a guest bus reset, a TDR-style reset) would do the heavy scrub.
    #[test]
    fn a_stale_mark_does_not_survive_a_pristine_completion() {
        let r = Reset::default();
        let o = Fake::default();
        r.request_typed(false, false); // the start-up reset
        assert_eq!(r.poll(&o), Progress::Completed(1));
        assert!(!r.scrub_owed(), "the start-up request left a mark");
        // the guest runs, then resets ITSELF through a bridge: no scrub
        let p = r.gate.enter().expect("open");
        r.touched();
        drop(p);
        r.request_typed(true, false);
        run_to_completion(&r, &o);
        assert_eq!(*o.scrubs.borrow(), [false], "a guest-issued reset must not scrub");
    }

    /// ★ A reboot of a guest that ran: the memory stage is asked to scrub, exactly once, and a second
    /// reboot with no guest access in between (nothing of a life exists) does not scrub again.
    #[test]
    fn a_reboot_scrubs_once_and_a_reboot_of_a_clean_store_does_not() {
        let r = Reset::default();
        let o = Fake::default();
        r.request_typed(false, false); // start-up
        run_to_completion(&r, &o);
        let p = r.gate.enter().expect("open");
        r.touched();
        drop(p);
        r.request_typed(false, false); // the guest reboots
        run_to_completion(&r, &o);
        assert_eq!(*o.scrubs.borrow(), [true]);
        assert!(!r.scrub_owed());
        r.request_typed(false, false); // reset button, no guest access since
        run_to_completion(&r, &o);
        assert_eq!(*o.scrubs.borrow(), [true], "a store no guest touched is not scrubbed again");
    }

    /// ★ A guest that ran (a dirty store) and resets ITSELF through a bridge, or wakes from S3, owes
    /// no scrub: the same OS keeps its root. (Kills a Memory stage that always asks to scrub.)
    #[test]
    fn a_guest_issued_reset_of_a_dirty_store_does_not_scrub() {
        let r = Reset::default();
        let o = Fake::default();
        let p = r.gate.enter().expect("open");
        r.touched();
        drop(p);
        r.request_typed(true, false); // a bus reset on a vCPU thread
        run_to_completion(&r, &o);
        assert_eq!(*o.scrubs.borrow(), [false]);
        let p = r.gate.enter().expect("open");
        r.touched();
        drop(p);
        r.request_typed(true, false);
        run_to_completion(&r, &o);
        assert_eq!(*o.scrubs.borrow(), [false, false], "never, however often the guest resets itself");
    }

    /// ★ Review B1(b): a reboot R2 that arrives while a vCPU-issued reset R1 is running must still
    /// scrub — at either point of R1's run (before its Memory stage is asked, or during it).
    #[test]
    fn a_reboot_during_a_guest_issued_reset_still_scrubs() {
        for during_memory in [false, true] {
            let r = Arc::new(Reset::default());
            let o = Fake::default();
            let r2 = r.clone();
            let hook: Box<dyn Fn()> = Box::new(move || {
                r2.request_typed(false, false); // R2: the guest reboots
            });
            if during_memory {
                *o.during_memory.borrow_mut() = Some(hook);
            } else {
                *o.during_release.borrow_mut() = Some(hook);
            }
            let p = r.gate.enter().expect("open");
            r.touched();
            drop(p);
            r.request_typed(true, false); // R1: a bus reset from a vCPU thread
            let end = run_to_completion(&r, &o);
            assert_eq!(end.last(), Some(&Progress::Completed(2)), "during_memory={during_memory}");
            assert!(
                o.scrubs.borrow().contains(&true),
                "during_memory={during_memory}: R2's new OS got no scrub: {:?}",
                o.scrubs.borrow()
            );
            assert!(!r.scrub_owed(), "during_memory={during_memory}: the mark leaked");
        }
    }

    /// ★ The machine's own reset at start-up (and a reset straight after a reset): no guest access
    /// was admitted, so there is nothing to undo — the gate opens at once and no owner is asked
    /// (the review of 2026-10-13: waiting for a VA thread busy with a cold prewarm would have kept
    /// the gate closed for seconds while the guest started).
    #[test]
    fn a_pristine_device_resets_at_once_without_asking_any_owner() {
        let r = Reset::default();
        let o = Fake::default();
        o.delay.set(u32::MAX);
        r.request();
        assert_eq!(r.poll(&o), Progress::Completed(1));
        assert!(o.calls.borrow().is_empty());
        assert!(!r.gate.is_closed() && !r.pending());
        assert!(r.status().contains("pristine=1"), "{}", r.status());
        // The first admitted access marks it; the next reset is a full one.
        let p = r.gate.enter().expect("open");
        r.touched();
        drop(p);
        r.request();
        assert_eq!(r.poll(&o), Progress::Pending(Stage::Release));
    }

    /// ★ The review of 2026-10-13 (HIGH): the device's stall report reads the status line, which
    /// read the stage under the sequence's own lock — held by `poll` while it reports: the drainer
    /// deadlocked exactly when a stage stalled. The stage is now mirrored lock-free.
    #[test]
    fn a_stall_report_may_read_the_status_without_deadlocking() {
        let r = Arc::new(Reset::default());
        let o = Fake::default();
        *o.status_of.borrow_mut() = Some(r.clone());
        o.delay.set(u32::MAX);
        r.touched();
        r.request();
        assert_eq!(r.poll(&o), Progress::Pending(Stage::Release));
        r.run.lock().expect("run").since =
            Some(std::time::Instant::now() - STALL_AFTER - std::time::Duration::from_millis(1));
        assert_eq!(r.poll(&o), Progress::Pending(Stage::Release));
        let seen = o.status_seen.borrow().clone().expect("the report read the status");
        assert!(seen.contains("stage=Release"), "{seen}");
    }

    /// ★ The review of 2026-10-13 (MEDIUM): Finish waits until no worker owns a token (the sweep
    /// would leave that word with its old route) — the gate stays closed meanwhile.
    #[test]
    fn finish_that_is_not_ready_keeps_the_gate_closed_until_it_is() {
        let r = Reset::default();
        let o = Fake::default();
        o.finish_polls.set(3);
        r.touched();
        r.request();
        for _ in 0..3 {
            assert_eq!(r.poll(&o), Progress::Pending(Stage::Finish));
            assert!(r.gate.is_closed());
        }
        assert_eq!(r.poll(&o), Progress::Completed(1));
        assert_eq!(
            *o.calls.borrow(),
            ["drainer 1", "release 1", "memory 1", "finish 1"]
        );
        assert!(!r.gate.is_closed());
    }

    /// Thread-safe owners for the bounded-wait tests: the drainer runs on its own thread, as in
    /// the device; `stuck_release` never acknowledges the Release stage.
    #[derive(Default)]
    struct SyncOwners {
        stuck_release: AtomicBool,
        release_polls: AtomicU64,
        finished: AtomicU64,
    }

    impl Owners for SyncOwners {
        fn reset_drainer_state(&self, _e: u64, _wake: bool) -> Life {
            Life::Ended
        }
        fn begin_release(&self, _e: u64) {
            self.release_polls.store(20, Ordering::SeqCst);
        }
        fn released(&self, _e: u64) -> bool {
            if self.stuck_release.load(Ordering::SeqCst) {
                return false;
            }
            // a few drainer passes, as the act thread's frees take
            let n = self.release_polls.load(Ordering::SeqCst);
            if n == 0 {
                return true;
            }
            self.release_polls.store(n - 1, Ordering::SeqCst);
            false
        }
        fn begin_memory(&self, _e: u64, _scrub: bool) {}
        fn memory_done(&self, _e: u64) -> bool {
            true
        }
        fn finish(&self, _e: u64, _life: Life) -> bool {
            self.finished.fetch_add(1, Ordering::SeqCst);
            true
        }
    }

    /// A drainer thread: polls every 100 µs until told to stop.
    fn drainer(
        r: Arc<Reset>,
        o: Arc<SyncOwners>,
        stop: Arc<AtomicBool>,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let _ = r.poll(&*o);
                std::thread::sleep(std::time::Duration::from_micros(100));
            }
        })
    }

    /// ★ The main loop's wait (QEMU's exit phase, vCPUs paused): it returns when the drainer
    /// completed the reset — every stage, the gate open — and the device is as realize left it
    /// before a single vCPU runs again.
    #[test]
    fn the_bounded_wait_returns_once_the_drainer_completed_the_reset() {
        let r = Arc::new(Reset::default());
        let o = Arc::new(SyncOwners::default());
        let stop = Arc::new(AtomicBool::new(false));
        let d = drainer(r.clone(), o.clone(), stop.clone());
        r.touched();
        let e = r.request_from(false);
        let t0 = std::time::Instant::now();
        assert_eq!(r.wait_completed(e, WAIT_BOUND), Waited::Done);
        assert!(t0.elapsed() < WAIT_BOUND);
        assert!(!r.gate.is_closed(), "the gate is open before the vCPUs resume");
        assert_eq!(o.finished.load(Ordering::SeqCst), 1);
        assert!(r.status().contains("main=1 vcpu=0"), "{}", r.status());
        // A pristine reset (the machine's own at creation) is done at once.
        let e = r.request_from(false);
        assert_eq!(r.wait_completed(e, WAIT_BOUND), Waited::Done);
        assert_eq!(o.finished.load(Ordering::SeqCst), 1, "no owner asked");
        stop.store(true, Ordering::SeqCst);
        d.join().expect("drainer");
    }

    /// ★ The degrade path: an owner that never acknowledges (a host free that hangs) cannot hold
    /// the VM. The wait returns at its bound naming the stage; the reset goes on asynchronously with
    /// the gate closed (fail closed) and completes when the owner finally answers.
    #[test]
    fn the_bounded_wait_gives_up_at_its_bound_and_the_reset_completes_later() {
        let r = Arc::new(Reset::default());
        let o = Arc::new(SyncOwners::default());
        o.stuck_release.store(true, Ordering::SeqCst);
        let stop = Arc::new(AtomicBool::new(false));
        let d = drainer(r.clone(), o.clone(), stop.clone());
        r.touched();
        let e = r.request_from(false);
        let bound = std::time::Duration::from_millis(50);
        let t0 = std::time::Instant::now();
        assert_eq!(r.wait_completed(e, bound), Waited::TimedOut(Stage::Release));
        assert!(t0.elapsed() >= bound && t0.elapsed() < bound * 20);
        assert!(r.gate.is_closed(), "past the bound the device stays closed");
        assert!(r.status().contains("waits_timed_out=1"), "{}", r.status());
        o.stuck_release.store(false, Ordering::SeqCst);
        assert_eq!(r.wait_completed(e, WAIT_BOUND), Waited::Done);
        assert!(!r.gate.is_closed());
        stop.store(true, Ordering::SeqCst);
        d.join().expect("drainer");
    }

    #[test]
    fn reset_lines_are_bounded_for_a_guest_that_resets_in_a_loop() {
        let n = (1..=1_000_000u64).filter(|n| loud(*n)).count();
        assert!(n <= 16 + 20, "{n} lines for a million resets");
        assert!(loud(1) && loud(16) && loud(1024) && !loud(17));
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

    // ---- ★ 2026-10-14: the S3 wake (`docs/design/V3_S3_RESUME.md`) ----------------------------

    fn touched_life(r: &Reset, o: &Fake) {
        r.touched();
        o.live_a_life(3);
    }

    /// ★★ A wake reset of a PM-suspended life keeps it: no Release, no Memory, its resources stay.
    #[test]
    fn a_wake_of_a_pm_suspended_life_keeps_it_and_skips_release_and_memory() {
        let r = Reset::default();
        let o = Fake::default();
        touched_life(&r, &o);
        o.pm_suspended.set(true);
        let e = r.request_typed(false, true);
        assert_eq!(run_to_completion(&r, &o).last(), Some(&Progress::Completed(e)));
        assert_eq!(
            *o.calls.borrow(),
            vec![format!("drainer-kept {e}"), format!("finish-kept {e}")]
        );
        assert_eq!(o.live.get(), 3, "the life's host resources are kept");
        assert!(!r.gate.is_closed(), "the gate opens");
        assert!(r.status().contains("kept=1"), "{}", r.status());
    }

    /// ⊘ The same wake without a PM suspend behind it is a full reset.
    #[test]
    fn a_wake_without_a_pm_suspend_is_a_full_reset() {
        let r = Reset::default();
        let o = Fake::default();
        touched_life(&r, &o);
        let e = r.request_typed(false, true);
        run_to_completion(&r, &o);
        assert_eq!(
            *o.calls.borrow(),
            vec![
                format!("drainer {e}"),
                format!("release {e}"),
                format!("memory {e}"),
                format!("finish {e}")
            ]
        );
        assert_eq!(o.live.get(), 0);
    }

    /// ⊘ A COLD reset of a PM-suspended life (QMP `system_reset` while suspended) ends it.
    #[test]
    fn a_cold_reset_of_a_pm_suspended_life_ends_it() {
        let r = Reset::default();
        let o = Fake::default();
        touched_life(&r, &o);
        o.pm_suspended.set(true);
        let e = r.request_from(false);
        run_to_completion(&r, &o);
        assert_eq!(o.calls.borrow()[0], format!("drainer {e}"));
        assert_eq!(o.live.get(), 0);
    }

    /// ⊘ A wake and a cold request covered by one run: cold wins.
    #[test]
    fn a_wake_and_a_cold_request_in_one_run_is_a_full_reset() {
        let r = Reset::default();
        let o = Fake::default();
        touched_life(&r, &o);
        o.pm_suspended.set(true);
        r.request_typed(false, true);
        let e = r.request_from(false);
        run_to_completion(&r, &o);
        assert_eq!(o.calls.borrow()[0], format!("drainer {e}"));
        assert_eq!(o.live.get(), 0);
    }

    /// ★ A cold reset right after a kept wake (nothing admitted in between) is NOT pristine: the
    /// kept life is still there and is ended.
    #[test]
    fn a_cold_reset_after_a_kept_wake_still_ends_the_life() {
        let r = Reset::default();
        let o = Fake::default();
        touched_life(&r, &o);
        o.pm_suspended.set(true);
        r.request_typed(false, true);
        run_to_completion(&r, &o);
        assert_eq!(o.live.get(), 3);
        let e = r.request_from(false);
        run_to_completion(&r, &o);
        assert!(o.calls.borrow().contains(&format!("release {e}")));
        assert_eq!(o.live.get(), 0);
    }

    /// ★ The machine's own cold reset at start-up finds the device pristine; its cold mark must not
    /// survive into the next life and turn that life's S3 wake into a full reset.
    #[test]
    fn a_pristine_cold_reset_does_not_leave_its_mark_on_the_next_wake() {
        let r = Reset::default();
        let o = Fake::default();
        r.request_from(false);
        run_to_completion(&r, &o);
        assert!(o.calls.borrow().iter().all(|c| c.starts_with("finish")));
        touched_life(&r, &o);
        o.pm_suspended.set(true);
        let e = r.request_typed(false, true);
        run_to_completion(&r, &o);
        assert!(o.calls.borrow().contains(&format!("drainer-kept {e}")));
        assert_eq!(o.live.get(), 3);
    }
}
