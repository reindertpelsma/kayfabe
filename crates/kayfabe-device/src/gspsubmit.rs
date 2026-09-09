//! ★★★★★ **w395 — THE GSP SUBMIT LANE: a guest `NV_PGSP_QUEUE_HEAD(0)` store is a KICK, not
//! a service pass.**
//!
//! ## §0 The ruling
//!
//! Owner, 2026-09-09: *"no inline blocking executions in mmio traps. just general rule. real
//! gpu also never holds any mmio write for milliseconds right. like rpc mmio starts the
//! operation, the block is for example a semaphore"* — and, of this register specifically:
//! *"submit register is a schedule, you should return to vm immediately and run it off the
//! vcpu threads"*.
//!
//! ## §1 What was measured, and why this is the critical path
//!
//! `[measured w394h, rev f365d831, GA106 / 580.159.04, fully armed]`
//! `TRAPWITNESS off_trap_claims=11491 inline_exceptions=51 worst_trap=1791581us
//! at=bar0+0x110c00 slow_traps(>1000us)=380`. `bar0+0x110c00` is `NV_PGSP_QUEUE_HEAD(0)`;
//! its write routed to `BootStep::CommandDoorbell` → `GspFsm::doorbell`, which drained and
//! serviced the whole GSP command ring — policy chain, host RM verbs and all — inside the
//! guest's store, under the BQL, with every vCPU stopped. **1.79 s**, and 380 traps over a
//! millisecond in one boot: a population, not an outlier.
//!
//! ## §2 Why a kick coalesces, and why that is CORRECT here where it was refuted for channels
//!
//! `w383` measured that coalescing channel doorbells is wrong, because the ring reader's
//! cursor is not re-read at execution. The GSP command queue is the opposite shape, and it is
//! the FSM's own code that says so: `GspFsm::drain_commands` reads the guest's `writePtr`
//! **fresh on every pass** and services every complete element from our `readPtr` up to it,
//! in ring order, committing the cursor per message. The `QUEUE_HEAD` *value* is not even
//! consulted (`GspReg::GspQueueHead(_) => CommandDoorbell`, `kayfabe_gsp::seq`). So N stores
//! before the worker wakes and one store after the last are **the same act**: one pass that
//! answers everything in the ring. Ordering lives in the ring, not in the kicks — a lane that
//! carries "there is work" as a level, not a count of tokens, cannot reorder or lose a command.
//!
//! ⚠ The one hazard of a level is the **lost wakeup** — a kick arriving while the worker is
//! mid-pass, folded into a pass that has already read `writePtr`. [`GspSubmitLane::kick`]
//! coalesces only into a kick the worker has **not yet taken**; a kick during a pass stays
//! pending and the worker goes round again. `the_kick_during_a_pass_is_not_lost` pins it.
//!
//! ## §3 What the lane is not
//!
//! It holds **no reference to the plane, the FSM or any isolate** — the same law
//! `kayfabe_rt::inbox` and [`crate::pubqueue`] state: a producer that wanted to touch core
//! state has nothing to touch. It carries nothing the guest wrote; the guest's data is in the
//! ring, where the FSM reads it under the plane's own lock.
//!
//! ⊘ There is no completion vector here and none is owed. The C shell **refuses**
//! `raise_status_irq` by name (`qemu/hw/misc/nvkvm/nvkvm.c:603`, `irq_requests_dropped`) and
//! the guest's `_kgspRpcRecvPoll` polls the response queue in its own RAM; the notification
//! the guest sees is *the reply landing in the ring*, and that is what the worker produces.
//! A worker that raised MSI-X 0 "for the status queue" would be a **new** notification the
//! control never sent, not a preserved one.

use std::sync::{Condvar, Mutex};
use std::time::Duration;

/// What [`GspSubmitLane::kick`] did — two facts, not a `bool`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kicked {
    /// The worker will wake for this kick.
    Queued,
    /// ★ A kick was already pending and untaken; this one folded into it. Not a drop: the
    /// pending pass reads `writePtr` when it runs and so covers this store too (§2).
    Coalesced,
}

/// One worker pass, as the lane accounts it. Carried by the caller, never derived here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PassFacts {
    /// Commands the pass answered.
    pub commands: u64,
    /// The pass ran the FSM's gate and found the queue **unbound** — a kick that outlived
    /// its binding (E12 on the worker). Counted, never silent.
    pub prebind: bool,
    /// The pass refused by name (`GspFault`). Counted; the fault text is the caller's to log.
    pub faulted: bool,
    /// The status-queue announcement the FSM asked for. **Not delivered** (module docs §3);
    /// counted so a boot can see the request rate the control also dropped.
    pub status_irq_requested: bool,
    /// How long the plane's mutex was held for this pass.
    pub hold: Duration,
}

/// Everything the lane can say about itself, as one value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LaneStats {
    /// Kicks that woke (or will wake) the worker.
    pub queued: u64,
    /// Kicks folded into an untaken pending kick.
    pub coalesced: u64,
    /// Wakes the worker took.
    pub taken: u64,
    /// Service passes the worker ran (each bounded by the caller's `max_commands`).
    pub passes: u64,
    /// Commands answered off the vCPU.
    pub commands: u64,
    /// Passes that found the queue unbound.
    pub prebind: u64,
    /// Passes that faulted.
    pub faults: u64,
    /// Status-queue announcements the FSM requested (and nobody delivered — §3).
    pub status_irq_requested: u64,
    /// Wakes fully drained (a pass answered zero commands, or faulted).
    pub completed: u64,
    /// The deepest the lane has been. A level's depth is 0 or 1; **2 means the invariant
    /// broke** and the census would say so.
    pub high_water: u64,
    /// The longest one pass held the plane's mutex.
    pub worst_hold_us: u64,
    /// The longest one wake took to drain, all passes summed.
    pub worst_drain_us: u64,
}

#[derive(Debug, Default)]
struct Inner {
    /// Kicks offered, monotonic. `kicks - taken` is the pending depth.
    kicks: u64,
    stats: LaneStats,
    stopping: bool,
}

/// ★★★ **The lane.** Many producers (every vCPU that traps on `QUEUE_HEAD`), one consumer,
/// a level rather than a queue, and a stop flag the consumer honours only once nothing is
/// pending — so a stop drains what was kicked rather than abandoning it.
#[derive(Debug, Default)]
pub struct GspSubmitLane {
    inner: Mutex<Inner>,
    wake: Condvar,
}

impl GspSubmitLane {
    /// An idle lane.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// ★★★★★ **THE vCPU-SIDE CALL, and the whole point: two integer compares under a leaf
    /// mutex and a `notify_one`.** No guest RAM, no host verb, nothing that can block on
    /// anything the guest must do.
    pub fn kick(&self) -> Kicked {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if g.kicks > g.stats.taken {
            // §2 — pending and untaken: the pass that will run covers this store.
            g.stats.coalesced += 1;
            return Kicked::Coalesced;
        }
        g.kicks += 1;
        g.stats.queued += 1;
        let depth = g.kicks - g.stats.taken;
        g.stats.high_water = g.stats.high_water.max(depth);
        drop(g);
        self.wake.notify_one();
        Kicked::Queued
    }

    /// Block until a kick is pending or the lane is stopping. `None` ⇒ stop.
    ///
    /// ⊘ Called **only** from the worker thread — a vCPU that called this would be waiting,
    /// under the BQL, on something the guest must do.
    ///
    /// ★ Takes **every** pending kick at once (`taken = kicks`): they are one act (§2). A
    /// kick that arrives after this returns is `kicks > taken` again and wakes the next
    /// wait — which is what makes a kick during a pass un-losable.
    pub fn wait_kick(&self) -> Option<u64> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if g.kicks > g.stats.taken {
                let n = g.kicks - g.stats.taken;
                g.stats.taken = g.kicks;
                return Some(n);
            }
            if g.stopping {
                return None;
            }
            g = self.wake.wait(g).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Non-blocking [`Self::wait_kick`]: take the pending kick if there is one.
    pub fn try_take(&self) -> Option<u64> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if g.kicks > g.stats.taken {
            let n = g.kicks - g.stats.taken;
            g.stats.taken = g.kicks;
            return Some(n);
        }
        None
    }

    /// The worker accounts one pass.
    pub fn note_pass(&self, pass: PassFacts) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.stats.passes += 1;
        g.stats.commands += pass.commands;
        g.stats.prebind += u64::from(pass.prebind);
        g.stats.faults += u64::from(pass.faulted);
        g.stats.status_irq_requested += u64::from(pass.status_irq_requested);
        let hold = u64::try_from(pass.hold.as_micros()).unwrap_or(u64::MAX);
        g.stats.worst_hold_us = g.stats.worst_hold_us.max(hold);
    }

    /// The worker announces one wake fully drained, and how long the drain took.
    pub fn note_completed(&self, drain: Duration) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.stats.completed += 1;
        let us = u64::try_from(drain.as_micros()).unwrap_or(u64::MAX);
        g.stats.worst_drain_us = g.stats.worst_drain_us.max(us);
        drop(g);
        self.wake.notify_all();
    }

    /// Tell the worker to finish what is pending and exit.
    pub fn stop(&self) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.stopping = true;
        drop(g);
        self.wake.notify_all();
    }

    /// Whether [`Self::stop`] has been called.
    #[must_use]
    pub fn stopping(&self) -> bool {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).stopping
    }

    /// Kicks pending and untaken right now — 0 or 1 for a level.
    #[must_use]
    pub fn depth(&self) -> u64 {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.kicks - g.stats.taken
    }

    /// This lane's numbers.
    #[must_use]
    pub fn stats(&self) -> LaneStats {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).stats
    }

    /// One line for a boot log. ⊘ Prints every field even when it is 0 — a lane that never
    /// carried anything and a lane that was never armed are different facts, and only the
    /// caller's `arm=` beside this line distinguishes them.
    #[must_use]
    pub fn census(&self) -> String {
        let s = self.stats();
        let depth = self.depth();
        format!(
            "GSPQUEUE queued={} coalesced={} taken={} passes={} commands={} prebind={} \
             faults={} status_irq_requested={} completed={} depth={depth} high_water={} \
             worst_hold_us={} worst_drain_us={}{}{}",
            s.queued,
            s.coalesced,
            s.taken,
            s.passes,
            s.commands,
            s.prebind,
            s.faults,
            s.status_irq_requested,
            s.completed,
            s.high_water,
            s.worst_hold_us,
            s.worst_drain_us,
            if s.high_water > 1 {
                " ⚠⚠ HIGH_WATER>1 — a level cannot be deeper than 1; the lane's invariant broke"
            } else {
                ""
            },
            if s.faults > 0 {
                " ⚠ FAULTS>0 — a worker pass refused by name; see the GSP-SUBMIT lines"
            } else {
                ""
            },
        )
    }
}

kayfabe_util::assert_send_sync!(GspSubmitLane);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kick_is_a_level_so_repeats_before_the_take_coalesce() {
        let lane = GspSubmitLane::new();
        assert_eq!(lane.kick(), Kicked::Queued);
        assert_eq!(lane.kick(), Kicked::Coalesced);
        assert_eq!(lane.kick(), Kicked::Coalesced);
        assert_eq!(lane.depth(), 1);
        assert_eq!(lane.try_take(), Some(1));
        assert_eq!(lane.depth(), 0);
        let s = lane.stats();
        assert_eq!((s.queued, s.coalesced, s.taken, s.high_water), (1, 2, 1, 1));
    }

    /// ★★★ §2's one hazard. A kick that lands AFTER the worker took the level — i.e. while
    /// a pass is reading the ring — must produce another wake, or the store it represents is
    /// serviced only by luck on the next unrelated doorbell.
    #[test]
    fn the_kick_during_a_pass_is_not_lost() {
        let lane = GspSubmitLane::new();
        lane.kick();
        assert_eq!(lane.try_take(), Some(1), "the worker takes the level");
        // the worker is now mid-pass; the guest rings again
        assert_eq!(lane.kick(), Kicked::Queued, "not coalesced into a pass already running");
        assert_eq!(lane.try_take(), Some(1), "the worker goes round again");
        assert_eq!(lane.try_take(), None);
    }

    #[test]
    fn stop_wakes_the_worker_only_once_nothing_is_pending() {
        let lane = std::sync::Arc::new(GspSubmitLane::new());
        lane.kick();
        lane.stop();
        // Pending work is taken BEFORE the stop is honoured.
        assert_eq!(lane.wait_kick(), Some(1));
        assert_eq!(lane.wait_kick(), None);
        assert!(lane.stopping());
    }

    #[test]
    fn a_blocked_worker_is_woken_by_a_kick() {
        let lane = std::sync::Arc::new(GspSubmitLane::new());
        let l = std::sync::Arc::clone(&lane);
        let worker = std::thread::spawn(move || l.wait_kick());
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(lane.kick(), Kicked::Queued);
        assert_eq!(worker.join().expect("worker"), Some(1));
    }

    #[test]
    fn the_census_carries_every_field_and_the_pass_facts_add_up() {
        let lane = GspSubmitLane::new();
        lane.kick();
        lane.try_take();
        lane.note_pass(PassFacts {
            commands: 3,
            prebind: false,
            faulted: false,
            status_irq_requested: true,
            hold: Duration::from_micros(250),
        });
        lane.note_pass(PassFacts {
            commands: 0,
            prebind: true,
            faulted: true,
            status_irq_requested: false,
            hold: Duration::from_micros(10),
        });
        lane.note_completed(Duration::from_micros(400));
        let c = lane.census();
        for want in [
            "GSPQUEUE queued=1",
            "coalesced=0",
            "taken=1",
            "passes=2",
            "commands=3",
            "prebind=1",
            "faults=1",
            "status_irq_requested=1",
            "completed=1",
            "depth=0",
            "high_water=1",
            "worst_hold_us=250",
            "worst_drain_us=400",
            "FAULTS>0",
        ] {
            assert!(c.contains(want), "{want:?} missing from {c:?}");
        }
        assert!(!c.contains("HIGH_WATER>1"));
    }
}
