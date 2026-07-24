//! # nvkvm-completion — the per-process completion plane
//!
//! The direct structural fix for #14's round-8 starvation localization
//! (`mode2_rust_rewrite_architecture.md` §4.3.2): in the C, completion delivery was
//! (a) invoked only from the doorbell handler, (b) gated on `any_completed`, and
//! (c) serialized behind one outstanding SWGEN0 batch — so a process that *submits
//! nothing but polls* never got its already-observed completion re-posted once the
//! other process went quiet.
//!
//! The rewrite splits the plane in two:
//!
//! - [`CompletionQueue`] — **per-`Proc`** pending/in-flight state. Observation is
//!   decoupled from delivery; nothing here is shared across processes.
//! - [`DeliveryPlane`] — the **device-global posting policy** over the (architecturally
//!   single) GSP status queue: one batch outstanding *per the queue's drain state*,
//!   composed from all procs' pending at post time. Crucially, [`DeliveryPlane::on_poll`]
//!   re-posts the **polling proc's own** pending — delivery is driven off the poller's
//!   RPC, not off any other process's doorbell. The round-8 starvation is impossible by
//!   construction (test `t14_per_proc_completion_no_starve`).
//!
//! Transport (seqNum ring discipline, the actual queue encoding) is `nvkvm-gsp`'s job;
//! this crate is pure policy and holds no NVIDIA layout.

use std::collections::VecDeque;

/// A reference to one guest-visible os-event completion (opaque to policy;
/// the GSP transport knows how to encode it on the queue).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OsEventRef(pub u64);

/// Identifies one posted SWGEN0 batch awaiting the guest's IRQSCLR drain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BatchId(pub u64);

/// Per-`Proc` completion state (`mode2_rust_rewrite_architecture.md` §4.3.2 layout).
///
/// Lifecycle of one event: `observe` → `pending` → (composed into a batch) →
/// `in_flight` → guest drains the batch → `awaiting_ack` → guest-visibly consumed
/// (`ack`) → gone. Any event still un-acked when its owner polls is **re-posted**
/// ([`CompletionQueue::take_unacked`]) — re-delivery is idempotent at this layer;
/// the guest-side os-event semantics tolerate re-posting (the C's poll-kick intent,
/// minus its gating flaw).
#[derive(Debug, Default)]
pub struct CompletionQueue {
    /// Observed, not yet composed into any batch.
    pending: VecDeque<OsEventRef>,
    /// Composed into the identified batch; the guest has not drained it yet.
    in_flight: Vec<(BatchId, OsEventRef)>,
    /// Batch drained by the guest, but the waiter has not provably consumed the
    /// event (no `ack`) — the re-post source on the owner's next poll.
    awaiting_ack: Vec<OsEventRef>,
}

impl CompletionQueue {
    /// Empty queue.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A completion was observed for this proc (host semaphore advance on a shared
    /// page, isolate `CoreEvent`, or — system proc only — a forge). Observation is
    /// decoupled from delivery: this never posts anything by itself.
    pub fn observe(&mut self, ev: OsEventRef) {
        self.pending.push_back(ev);
    }

    /// True if this proc has anything undelivered or un-acked.
    #[must_use]
    pub fn has_outstanding(&self) -> bool {
        !self.pending.is_empty() || !self.in_flight.is_empty() || !self.awaiting_ack.is_empty()
    }

    /// Move all `pending` into `in_flight` under `batch`; returns the events to post.
    /// Called only by [`DeliveryPlane`] at a post point.
    pub fn compose_into(&mut self, batch: BatchId) -> Vec<OsEventRef> {
        let evs: Vec<OsEventRef> = self.pending.drain(..).collect();
        self.in_flight.extend(evs.iter().map(|&e| (batch, e)));
        evs
    }

    /// The guest drained `batch` (IRQSCLR): its events move to `awaiting_ack`.
    pub fn drained(&mut self, batch: BatchId) {
        let mut kept = Vec::with_capacity(self.in_flight.len());
        for (b, e) in self.in_flight.drain(..) {
            if b == batch {
                self.awaiting_ack.push(e);
            } else {
                kept.push((b, e));
            }
        }
        self.in_flight = kept;
    }

    /// The guest provably consumed `ev` (its waiter woke / it unregistered).
    pub fn ack(&mut self, ev: OsEventRef) {
        self.awaiting_ack.retain(|&e| e != ev);
        self.pending.retain(|&e| e != ev);
        self.in_flight.retain(|&(_, e)| e != ev);
    }

    /// Requeue everything drained-but-unacked as pending (the re-post source when
    /// the owner polls). Returns how many were requeued.
    pub fn take_unacked(&mut self) -> usize {
        let n = self.awaiting_ack.len();
        for e in self.awaiting_ack.drain(..) {
            self.pending.push_back(e);
        }
        n
    }
}

/// What the caller must do after a policy decision: post these events as one GSP
/// batch and raise one SWGEN0 edge (transport = `nvkvm-gsp`; irq = `Vmm::raise_irq`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostBatch {
    /// The batch to encode on the shared GSP status queue.
    pub batch: BatchId,
    /// Events composed into it (may span several procs — one drain carries
    /// independent completions; no cross-process serialization).
    pub events: Vec<OsEventRef>,
}

/// Device-global posting policy over the single shared GSP status queue.
///
/// The queue is architecturally one (one faked GSP per VM, one seqNum stream — the
/// ⚠8 *transport* constraint is real and honored at the single post point). This
/// policy keeps **one batch outstanding per the queue's drain state** while letting
/// every proc's completions ride any batch, and re-posts on the owner's own poll.
#[derive(Debug, Default)]
pub struct DeliveryPlane {
    outstanding: Option<BatchId>,
    next_batch: u64,
}

impl DeliveryPlane {
    /// Fresh plane with no batch outstanding.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// True if a posted batch has not yet been drained by the guest.
    #[must_use]
    pub fn batch_outstanding(&self) -> bool {
        self.outstanding.is_some()
    }

    /// Try to post: compose one batch from every queue in `queues` that has pending
    /// events. Returns `None` if the drain gate is closed (a batch is outstanding —
    /// over-posting desyncs the seqNum ring, lesson L10) or nothing is pending.
    pub fn try_post<'q>(
        &mut self,
        queues: impl IntoIterator<Item = &'q mut CompletionQueue>,
    ) -> Option<PostBatch> {
        if self.outstanding.is_some() {
            return None;
        }
        let batch = BatchId(self.next_batch);
        let mut events = Vec::new();
        for q in queues {
            events.extend(q.compose_into(batch));
        }
        if events.is_empty() {
            return None;
        }
        self.next_batch += 1;
        self.outstanding = Some(batch);
        Some(PostBatch { batch, events })
    }

    /// The guest drained the outstanding batch (IRQSCLR observed). Marks every
    /// queue's in-flight events for that batch as awaiting ack and opens the gate.
    pub fn drained<'q>(&mut self, queues: impl IntoIterator<Item = &'q mut CompletionQueue>) {
        if let Some(batch) = self.outstanding.take() {
            for q in queues {
                q.drained(batch);
            }
        }
    }

    /// ★ The starvation fix: the owning proc polled (`MC_SERVICE_INTERRUPTS`-shaped
    /// RPC). Requeue its drained-but-unacked events and try to post — driven off
    /// **the poller's own RPC**, never another process's doorbell. `others` are the
    /// remaining procs' queues (their pending may ride the same batch).
    pub fn on_poll<'q>(
        &mut self,
        poller: &'q mut CompletionQueue,
        others: impl IntoIterator<Item = &'q mut CompletionQueue>,
    ) -> Option<PostBatch> {
        poller.take_unacked();
        if !poller.has_outstanding() {
            return None;
        }
        self.try_post(core::iter::once(poller).chain(others))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The round-8 starvation shape, at unit speed: proc A polls but never submits;
    /// proc B (the "winner") goes quiet. A's completion must still be (re)delivered,
    /// driven purely off A's own poll.
    #[test]
    fn t14_per_proc_completion_no_starve() {
        let mut plane = DeliveryPlane::new();
        let mut qa = CompletionQueue::new();
        let mut qb = CompletionQueue::new();

        // B submits and completes; a batch containing B's event posts and drains.
        qb.observe(OsEventRef(0xb0));
        let post = plane.try_post([&mut qa, &mut qb]).expect("B's completion posts");
        assert_eq!(post.events, vec![OsEventRef(0xb0)]);
        plane.drained([&mut qa, &mut qb]);
        qb.ack(OsEventRef(0xb0));

        // A's completion is observed... but composed into a batch that the guest
        // drains WITHOUT A's waiter waking (the lost-wakeup window).
        qa.observe(OsEventRef(0xa0));
        let post = plane.try_post([&mut qa, &mut qb]).expect("A's completion posts");
        assert_eq!(post.events, vec![OsEventRef(0xa0)]);
        plane.drained([&mut qa, &mut qb]);
        // No ack from A. B rings no more doorbells — in the C, delivery is dead here.

        // A polls: its un-acked event is re-posted off its OWN poll.
        let repost = plane.on_poll(&mut qa, [&mut qb]).expect("poll re-posts A's event");
        assert_eq!(repost.events, vec![OsEventRef(0xa0)]);
        plane.drained([&mut qa, &mut qb]);
        qa.ack(OsEventRef(0xa0));
        assert!(!qa.has_outstanding());
        // And a poll with nothing outstanding posts nothing (no over-posting).
        assert!(plane.on_poll(&mut qa, [&mut qb]).is_none());
    }

    /// The drain gate: one batch outstanding; composing is refused until drained
    /// (the seqNum-ring transport constraint) — but ONE batch may carry BOTH procs'
    /// independent completions (no cross-process serialization).
    #[test]
    fn drain_gate_batches_across_procs_without_serializing() {
        let mut plane = DeliveryPlane::new();
        let mut qa = CompletionQueue::new();
        let mut qb = CompletionQueue::new();
        qa.observe(OsEventRef(1));
        qb.observe(OsEventRef(2));
        let post = plane.try_post([&mut qa, &mut qb]).unwrap();
        assert_eq!(post.events, vec![OsEventRef(1), OsEventRef(2)], "one batch, both procs");
        // Gate closed while outstanding.
        qa.observe(OsEventRef(3));
        assert!(plane.try_post([&mut qa, &mut qb]).is_none(), "gate closed until drain");
        plane.drained([&mut qa, &mut qb]);
        let post = plane.try_post([&mut qa, &mut qb]).unwrap();
        assert_eq!(post.events, vec![OsEventRef(3)]);
    }
}
