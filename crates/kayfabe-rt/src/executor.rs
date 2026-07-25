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
