//! The executor inbox — **the one concurrent L1 structure** (rank 2, leaf;
//! `l1_concurrency.md` §2/§9).
//!
//! Producers are the reactor loop and the defer path; the consumer is the
//! executor. **Inherited law 9 is encoded in the types, not just documented:**
//! the producer side is [`InboxSender`], which holds the queue and *nothing
//! else* — it has no reference to the device, so a producer that wanted to touch
//! core state has nothing to touch. (The §9 contract row for the reactor loop:
//! "touches ZERO core state — enforced by giving it no reference to the device,
//! only the inbox sender".)
//!
//! Rank 2 matters both ways: a core path holding the device (and possibly a proc)
//! lock may legally push a deferred event — rank 0/1 → rank 2 is strictly
//! increasing — while the executor's pop must scope its rank-2 guard closed
//! *before* it acquires the device lock (rank 2 → rank 0 would be an R3 panic).
//! [`Inbox::try_pop`] scopes the guard internally, so the discipline is
//! structural.
//!
//! Capacity is unbounded `VecDeque` — per the §9 contract this is not a
//! guest-amplifiable surface: events are per-source-signal / per-deadline, never
//! per-guest-byte.
//!
//! Stage 2 deliberately provides **no blocking wait** (no condvar): the OS reactor
//! loop and the executor's park/wake are stage 3 / L2. The seam is
//! [`Inbox::try_pop`] returning `None` — the stage-3 shell wraps it with its real
//! waiting primitive.

use std::collections::VecDeque;
use std::sync::Arc;

use kayfabe_core::reactor::CompletionSource;
use kayfabe_vmm::CoreEventKind;

use crate::lock::{LockRank, RankedMutex};

/// An event bound for the serialized executor. Small on purpose; stage 3 extends
/// it (and decides how `kayfabe_vmm::CoreEvent` — the defer-callback enum this
/// deliberately mirrors — folds in).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreEvent {
    /// A completion source signalled (the reactor loop's product: real readiness
    /// mapped to the opaque handle by a pure table lookup — no payload, no
    /// interpretation; dispatch is the core's job on the executor).
    SourceSignal(CompletionSource),
    /// A deferred-work deadline fired (`Vmm::defer`'s discriminant).
    Deferred(CoreEventKind),
    /// Placeholder for stage 3's async isolate completion (the
    /// plan/execute/commit re-entry seam, R1's consequence in §3.3). Carried now
    /// so the inbox's shape does not change when the blocking verb path lands.
    IsolateComplete {
        /// The isolate's session id (== ProcId by construction).
        session: u64,
        /// The typed continuation cookie the core supplied at issue.
        cookie: u64,
    },
}

/// The shared queue (rank 2). Private: only the two typed ends below reach it.
struct Shared {
    q: RankedMutex<VecDeque<CoreEvent>>,
}

/// The producer end. Cloneable (many producers), and — law 9 — structurally
/// unable to touch core state: it owns an `Arc` of the queue and nothing else.
#[derive(Clone)]
pub struct InboxSender {
    shared: Arc<Shared>,
}

impl InboxSender {
    /// Enqueue one event. Acquires only the rank-2 leaf lock, so it is legal from
    /// a bare producer thread (zero locks) AND from under the device/proc locks
    /// (strictly increasing rank).
    pub fn send(&self, ev: CoreEvent) {
        self.shared.q.lock().push_back(ev);
    }
}

/// The consumer end (the executor's). Not cloneable — one drain authority.
pub struct Inbox {
    shared: Arc<Shared>,
}

impl Inbox {
    /// Pop the oldest event, if any. The rank-2 guard is scoped INSIDE this call:
    /// on return the thread holds nothing, so the executor's subsequent device
    /// acquisition starts from rank-clean state (R3, module docs).
    pub fn try_pop(&self) -> Option<CoreEvent> {
        self.shared.q.lock().pop_front()
    }

    /// Mint an additional producer handle.
    #[must_use]
    pub fn sender(&self) -> InboxSender {
        InboxSender {
            shared: Arc::clone(&self.shared),
        }
    }

    /// Events currently queued.
    #[must_use]
    pub fn len(&self) -> usize {
        self.shared.q.lock().len()
    }

    /// True if nothing is queued.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Build the inbox: one producer handle (clone for more) and the consumer end.
#[must_use]
pub fn inbox() -> (InboxSender, Inbox) {
    let shared = Arc::new(Shared {
        q: RankedMutex::new(LockRank::Leaf, VecDeque::new()),
    });
    (
        InboxSender {
            shared: Arc::clone(&shared),
        },
        Inbox { shared },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::{self, LockRank, RankedMutex, RankedRwLock};

    /// FIFO per producer; the consumer sees sends in order; `try_pop` on empty is
    /// `None` (the stage-3 wait seam).
    #[test]
    fn inbox_is_fifo_and_try_pop_is_nonblocking() {
        let (tx, rx) = inbox();
        assert!(rx.try_pop().is_none());
        tx.send(CoreEvent::Deferred(CoreEventKind::DeferredReap));
        rx.sender().send(CoreEvent::IsolateComplete {
            session: 7,
            cookie: 9,
        });
        assert_eq!(rx.len(), 2);
        assert_eq!(
            rx.try_pop(),
            Some(CoreEvent::Deferred(CoreEventKind::DeferredReap))
        );
        assert_eq!(
            rx.try_pop(),
            Some(CoreEvent::IsolateComplete {
                session: 7,
                cookie: 9
            })
        );
        assert!(rx.is_empty());
    }

    /// The rank discipline around the inbox, both ways: sending from under
    /// device+proc locks is legal (rank strictly increases), and `try_pop`
    /// releases rank 2 before returning (depth back to the caller's).
    #[test]
    fn inbox_send_is_legal_under_higher_locks_and_pop_releases_rank() {
        let (tx, rx) = inbox();
        let device = RankedRwLock::new(LockRank::Device, ());
        let proc = RankedMutex::new(LockRank::Proc, ());
        {
            let _d = device.read();
            let _p = proc.lock();
            tx.send(CoreEvent::Deferred(CoreEventKind::CompletionRedeliver));
            assert_eq!(lock::held_depth(), 2, "send left only the caller's locks");
        }
        assert_eq!(lock::held_depth(), 0);
        assert!(rx.try_pop().is_some());
        assert_eq!(lock::held_depth(), 0, "try_pop scoped its rank-2 guard");
    }
}
