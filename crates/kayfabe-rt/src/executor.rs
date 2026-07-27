//! The serialized executor (`l1_concurrency.md` §2): drains the [`CoreEvent`]
//! inbox in order and runs each event against the [`SharedDevice`] **under the
//! same locks and the same R1–R5 discipline as a vCPU thread would** — the
//! executor has no private door into the core. Asynchronous isolate I/O completes
//! here, never by re-entry from an isolate or reactor thread (inherited law 9).
//!
//! Stage 2 is the pure-`std` shell: there is no loop thread and no parking —
//! [`Executor::drain_one`] is a plain function the tests (and stage 3's real loop)
//! call. The seam to the OS is deliberately obvious: stage 3 wraps `drain_one` in
//! "wait for the notifiable source, then drain".

use std::sync::Arc;

use kayfabe_arch::ids::GpuId;
use kayfabe_completion::PostBatch;
use kayfabe_vmm::CoreEventKind;

use crate::device::{SharedDevice, SignalOutcome};
use crate::inbox::{CoreEvent, Inbox};

/// What draining one [`CoreEvent`] did — typed so the caller (tests now, the
/// stage-3 shell later) can act on it; nothing is silently swallowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// A source signal was dispatched and applied (or loudly refused) — see
    /// [`SignalOutcome`].
    Signal(SignalOutcome),
    /// A `DeferredReap` deadline ran `reap_retired` at this quiesce point
    /// (inherited law 8); carries the number reaped.
    Reaped(usize),
    /// A `CompletionRedeliver` sweep ran: the per-target pump edges, in ascending
    /// target order; carries any batches posted. **The caller owns delivering
    /// them** (GSP-queue encode + IRQ — the `Vmm` seam, stage 3): an undelivered
    /// batch must be fed back via `completions_drained`-after-delivery exactly as
    /// a real drain would, which is why the batches are surfaced, not dropped.
    Redelivered(Vec<(GpuId, PostBatch)>),
    /// A deferred kind stage 2 does not wire (`PollKickBudget`, `RegionFault`) —
    /// surfaced untouched so the gap is visible, never guessed at.
    Deferred(CoreEventKind),
    /// Stage 3's isolate-completion placeholder, surfaced untouched (the
    /// plan/execute/commit commit phase does not exist yet).
    IsolateComplete {
        /// The isolate's session id.
        session: u64,
        /// The continuation cookie.
        cookie: u64,
    },
}

/// The executor: the [`Inbox`] consumer plus its device. Owns the only [`Inbox`]
/// (one drain authority); producers hold [`crate::inbox::InboxSender`]s and — by
/// type — nothing of the device.
pub struct Executor {
    device: Arc<SharedDevice>,
    inbox: Inbox,
}

impl Executor {
    /// Build the executor over `device`, consuming the inbox's receive end.
    #[must_use]
    pub fn new(device: Arc<SharedDevice>, inbox: Inbox) -> Self {
        Executor { device, inbox }
    }

    /// The device this executor drives (for callers that hold only the executor).
    #[must_use]
    pub fn device(&self) -> &Arc<SharedDevice> {
        &self.device
    }

    /// Pop and run ONE event; `None` when the inbox is empty. The inbox's rank-2
    /// guard is released before any device lock is taken (R3 — see
    /// [`Inbox::try_pop`]), so each event runs from rank-clean state.
    pub fn drain_one(&mut self) -> Option<Effect> {
        let ev = self.inbox.try_pop()?;
        Some(match ev {
            CoreEvent::SourceSignal(source) => Effect::Signal(self.device.signal_source(source)),
            CoreEvent::Deferred(CoreEventKind::DeferredReap) => {
                Effect::Reaped(self.device.reap_retired())
            }
            CoreEvent::Deferred(CoreEventKind::CompletionRedeliver) => {
                // The bounded backstop edge (§5.2): pump each realized target
                // once. Ascending-target iteration order via the snapshot the
                // device exposes is not needed — GpuId::ZERO is always realized
                // and further targets are pumped by their own edges; stage 2
                // pumps the default target (multi-target backstop cadence is a
                // stage-3 policy decision, taken with the defer plumbing).
                let batches = self
                    .device
                    .pump_completions(GpuId::ZERO)
                    .map(|b| vec![(GpuId::ZERO, b)])
                    .unwrap_or_default();
                Effect::Redelivered(batches)
            }
            CoreEvent::Deferred(kind) => Effect::Deferred(kind),
            CoreEvent::IsolateComplete { session, cookie } => {
                Effect::IsolateComplete { session, cookie }
            }
        })
    }

    /// Drain until empty, returning every effect in drain order.
    pub fn drain_all(&mut self) -> Vec<Effect> {
        let mut out = Vec::new();
        while let Some(e) = self.drain_one() {
            out.push(e);
        }
        out
    }
}

// =====================================================================================
// ★ §3.7 — the executor is its own thread, and its wake is abstracted NOW
// =====================================================================================

/// ★ How the executor is told there is work — `l1_os_shell.md` §3.7.
///
/// > *"Four lines, and it is the difference between L2 being an adapter and L2 being a
/// > re-plumb of the inbox. This is the cheapest anti-bolt-on in the doc; take it."*
///
/// The §3.7 amendment (2026-07-26) settled that both backends can use the **same**
/// primitive — under the ≥ 10.2 floor neither QEMU entry path arrives holding a foreign
/// lock — and then said the trait survives the amendment intact, *"which is the point of
/// having abstracted it: the argument for it was never that the two impls differ."* So
/// this stays a trait, with one implementation ([`Parker`]) and a named seam.
pub trait ExecutorWaker: Send + Sync + core::fmt::Debug {
    /// Tell the executor there is work. **Must not block and must not fail** — it is
    /// called from the reactor loop between two readiness reports.
    fn wake(&self);
}

/// The harness's [`ExecutorWaker`]: a condition variable and a monotonic wake sequence.
///
/// ## ★ Why the sequence number, and not just the condvar
///
/// A condvar alone loses the wake that lands between "the executor found the inbox empty"
/// and "the executor parked" — the classic lost-wakeup, which here presents as a hung
/// device and an idle CPU, i.e. exactly the F3 class this project exists to prevent. The
/// sequence is the state the wait predicate is over: [`Parker::park`] takes the sequence
/// the caller last observed and returns only when it has moved (or the deadline passes,
/// or shutdown is requested). A wake that arrives early bumps the sequence, so the park
/// returns immediately rather than waiting for a wake that already happened.
#[derive(Debug, Default)]
pub struct Parker {
    state: std::sync::Mutex<ParkState>,
    signal: std::sync::Condvar,
}

#[derive(Debug, Default, Clone, Copy)]
struct ParkState {
    seq: u64,
    stopping: bool,
}

impl Parker {
    /// A fresh parker, not stopping.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The current wake sequence — read it *before* draining, pass it to [`Parker::park`]
    /// after, and no wake can fall in the gap.
    #[must_use]
    pub fn sequence(&self) -> u64 {
        self.state.lock().expect("parker").seq
    }

    /// Ask the executor to stop and wake it. Idempotent.
    pub fn stop(&self) {
        let mut st = self.state.lock().expect("parker");
        st.stopping = true;
        st.seq += 1;
        drop(st);
        self.signal.notify_all();
    }

    /// True once [`Parker::stop`] has been called.
    #[must_use]
    pub fn is_stopping(&self) -> bool {
        self.state.lock().expect("parker").stopping
    }

    /// Park until the wake sequence moves past `observed`, `timeout` elapses, or shutdown
    /// is requested. Returns the sequence now current.
    ///
    /// # Panics
    /// If called with any ranked lock or any adapter leaf lock held: parking is the
    /// single most blocking thing the executor does, and R1 admits no exception for it.
    pub fn park(&self, observed: u64, timeout: core::time::Duration) -> u64 {
        kayfabe_util::lockwitness::assert_lock_free("parking the executor");
        kayfabe_util::leafwitness::assert_leaf_free("parking the executor");
        let st = self.state.lock().expect("parker");
        let (st, _) = self
            .signal
            .wait_timeout_while(st, timeout, |s| s.seq == observed && !s.stopping)
            .expect("parker");
        st.seq
    }
}

impl ExecutorWaker for Parker {
    fn wake(&self) {
        let mut st = self.state.lock().expect("parker");
        st.seq += 1;
        drop(st);
        self.signal.notify_all();
    }
}

impl Executor {
    /// ★ The executor's own loop: drain everything, then park until woken.
    ///
    /// The order is the whole lost-wakeup argument and is not an implementation detail:
    /// **read the sequence, drain, park on the sequence you read.** A wake that arrives
    /// during the drain has already moved the sequence, so the park returns at once.
    ///
    /// `on_effect` is called for every drained event, on this thread, with no lock held —
    /// which is where the §6.5 delivery tail (encode → `gpa_write` → IRQ) belongs.
    pub fn run_until_stopped(
        &mut self,
        parker: &Parker,
        poll_interval: core::time::Duration,
        mut on_effect: impl FnMut(Effect),
    ) {
        loop {
            let seq = parker.sequence();
            for effect in self.drain_all() {
                on_effect(effect);
            }
            if parker.is_stopping() {
                // ★ One final drain AFTER observing the stop: an event pushed between the
                // drain above and the stop flag being read would otherwise be abandoned,
                // and §7's teardown ledger counts events that name dead procs (R9).
                for effect in self.drain_all() {
                    on_effect(effect);
                }
                return;
            }
            parker.park(seq, poll_interval);
        }
    }
}
