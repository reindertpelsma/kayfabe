//! # kayfabe-completion — the per-process completion plane
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
//! Transport (seqNum ring discipline, the actual queue encoding) is `kayfabe-gsp`'s job;
//! this crate is pure policy and holds no NVIDIA layout.
//!
//! Concurrency (decision #17): plain owned data, no interior mutability; all
//! mutation is `&mut self` — see `kayfabe-core`'s crate docs for the full contract.
//! `Send + Sync` compile-time-asserted below.

use std::collections::{BTreeMap, VecDeque};

/// A reference to one guest-visible os-event completion (opaque to policy;
/// the GSP transport knows how to encode it on the queue).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OsEventRef(pub u64);

/// Identifies one posted SWGEN0 batch awaiting the guest's IRQSCLR drain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BatchId(pub u64);

/// Maximum completions one process may hold outstanding (pending + in-flight +
/// awaiting-ack) before [`CompletionQueue::observe`] loud-faults.
///
/// **Boundary-1: no unbounded allocation from guest input.** A hostile guest can
/// trigger completions (pushbuffer `SEM_RELEASE`s, present vblanks) far faster than
/// it drains them; without this bound one process's `pending` queue would grow until
/// the host OOM-aborts — taking every *other* guest process down with it. The bound
/// is orders of magnitude above any legitimate outstanding depth, so a real workload
/// never trips it; only a flood does, and it gets a loud [`CompletionError::QueueFull`].
pub const MAX_OUTSTANDING_COMPLETIONS: usize = 1 << 18;

/// Maximum mapped-fence arms one process may hold concurrently armed before
/// [`FenceArms::arm`] loud-faults (boundary-1 — same rationale as
/// [`MAX_OUTSTANDING_COMPLETIONS`]: a hostile guest must not grow core state
/// without bound; a real workload arms a handful of fences).
pub const MAX_ARMED_FENCES: usize = 1 << 16;

/// ★ The #12 semaphore-jump guard, on the fence arm (pattern **e**).
///
/// The proven failure shape (`mode2_12_layered_status` cont.32/33, UVM
/// `uvm_gpu_semaphore.c:776`): a stale/foreign write made a tracking semaphore
/// appear to jump BACKWARDS — under wrap arithmetic that is an absurdly large
/// forward jump — and the guest's own guard (`MAX_JUMP = 2 × GPFIFO entries`)
/// went fatal. The core applies the same discipline at observation time: a fence
/// value that steps further than this bound from the last observed value is a
/// loud [`CompletionError::FenceJump`] refusal, **never** treated as a
/// completion. Mirrors the guest's own bound (2 × max GPFIFO entries), abstract
/// here — no NVIDIA layout.
pub const MAX_FENCE_JUMP: u64 = 2 * 1024;

/// A loud completion-plane fault (boundary-1). The queue/arms are left unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionError {
    /// The per-process completion queue is at [`MAX_OUTSTANDING_COMPLETIONS`]; the
    /// observation is refused rather than growing the queue unboundedly.
    QueueFull,
    /// The per-process fence-arm table is at [`MAX_ARMED_FENCES`].
    FenceTableFull,
    /// A second, *different* arm for a fence key that is already armed (an
    /// identical re-send is idempotent — the retried-RPC discipline).
    FenceRearm {
        /// The colliding fence key `(vas, addr)`.
        key: (u64, u64),
    },
    /// An observed fence value stepped further than [`MAX_FENCE_JUMP`] from the
    /// last observed value — a backwards/absurd jump (a stale or foreign write,
    /// the #12 class). Refused loudly; the arm state is unchanged.
    FenceJump {
        /// Last accepted raw fence value.
        last: u32,
        /// The refused observed value.
        value: u32,
    },
}

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
    ///
    /// Loud-faults with [`CompletionError::QueueFull`] once the process holds
    /// [`MAX_OUTSTANDING_COMPLETIONS`] undrained completions (boundary-1: a hostile
    /// guest cannot grow this queue without bound). The queue is left unchanged on
    /// refusal.
    ///
    /// # ⊘⊘⊘ THIS QUEUE HAS NO PRODUCTION DRAIN, AND THE RATE IS MEASURED
    ///
    /// `[measured 2026-08-21, by census of the tree]` **The fill side of this plane is
    /// wired and the delivery side is not.** `observe` has **23 production call sites**
    /// (`kayfabe-fwd`, one per completion). [`CompletionQueue::ack`] — the *only* method
    /// that removes anything — has **zero**; so do `compose_into`, `confirm`,
    /// `take_unacked`, `has_outstanding` and `outstanding_len`. Every non-test `.ack(` in
    /// the tree is an unrelated `cmd.ack(0)` in a rustdoc comment.
    /// ⊘ `take_unacked` is **not** a drain — it moves `awaiting_ack` back to `pending`, and
    /// [`CompletionQueue::outstanding_len`] is the **sum** of all three, so composition and
    /// requeue move items between buckets without ever reducing the total.
    ///
    /// ⇒ In production `outstanding_len()` is **monotonically non-decreasing**, and at
    /// [`MAX_OUTSTANDING_COMPLETIONS`] = 262 144 this function starts returning
    /// [`CompletionError::QueueFull`] — which `?` turns into a forwarding fault that fails
    /// the whole pushbuffer apply, i.e. **refuses every subsequent CE doorbell for that
    /// process, permanently.** The queue is per-`Proc`, so a short-lived process never
    /// reaches it and **a long-lived one always does**.
    ///
    /// ★★★ **The workload that reaches this is the project's own north star**: a sustained
    /// inference loop. And the test written to model exactly that — `soak_llm_like.rs` —
    /// **acks every token**, so the one test built to catch this structurally cannot.
    ///
    /// ⚠ **The deferral is deliberate and defensible; this note is its unpaid half.** Not
    /// driving `DeliveryPlane::on_poll` is an explicit, argued decision with two measured
    /// reasons (`kayfabe-rmrpc/src/policy.rs`, *"what this arm deliberately does NOT do —
    /// and it is half the rung"*). What that decision did **not** cost is what happens when
    /// the still-wired producer runs 262 144 times against a consumer that was deferred.
    /// ⇒ Same shape as **PC-D7** (`kayfabe-gsp/src/boot.rs`): a hazard whose *mechanism* was
    /// reviewed and whose *rate* was not, filed as accepted, and catastrophic in practice.
    /// **A deferral is a claim about consequences, not only about intent.**
    pub fn observe(&mut self, ev: OsEventRef) -> Result<(), CompletionError> {
        if self.outstanding_len() >= MAX_OUTSTANDING_COMPLETIONS {
            return Err(CompletionError::QueueFull);
        }
        self.pending.push_back(ev);
        Ok(())
    }

    /// Total outstanding completions (pending + in-flight + awaiting-ack) — the
    /// quantity [`MAX_OUTSTANDING_COMPLETIONS`] bounds.
    #[must_use]
    pub fn outstanding_len(&self) -> usize {
        self.pending.len() + self.in_flight.len() + self.awaiting_ack.len()
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

/// One armed mapped-fence completion (pattern **e** internals; see [`FenceArms`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FenceArm {
    /// Remaining distance (in fence-value steps, wrap-corrected) to the target.
    to_go: u64,
    /// Last accepted raw 32-bit fence value (the wrap-guard reference point).
    last: u32,
    /// The declared target payload (kept for idempotent re-arm detection).
    target: u32,
    /// The completion identity handed back when the fence fires.
    event: OsEventRef,
}

/// ★ Completion pattern **(e)** — the per-`Proc` **mapped-fence** arms
/// (`execution_plane.md` §1.2/§2.4; the NVENC fence-not-event shape, `nvenc_101`:
/// the worker reads a GPU-written mapped fence with NO syscall).
///
/// Deliberately **distinct from the event-delivery path**: a fired fence never
/// enters a [`CompletionQueue`], never composes into a [`DeliveryPlane`] batch,
/// never raises SWGEN0 — the guest observes the value straight from the mapped
/// page (like pattern (a)'s passthrough poll). What the core owns is the *arming
/// semantics*: which `(vas, addr)` a channel's completion rides, when the observed
/// value counts as "at/after target" under 32-bit wrap, and the #12 jump guard
/// ([`MAX_FENCE_JUMP`]) that refuses stale/backwards writes loudly.
///
/// Keys are abstract `(vas, addr)` pairs — the fwd plane keys them by
/// `(Pdb, GpuVa)` (address ops key on the `Vas`, decision #14); this crate stays
/// dependency-free and holds no NVIDIA layout.
#[derive(Debug, Default)]
pub struct FenceArms {
    armed: BTreeMap<(u64, u64), FenceArm>,
}

impl FenceArms {
    /// No arms.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of currently armed fences (bounded by [`MAX_ARMED_FENCES`]).
    #[must_use]
    pub fn armed_len(&self) -> usize {
        self.armed.len()
    }

    /// Arm a fence at `key = (vas, addr)`: fire once the mapped value is observed
    /// at/after `target`, starting from the currently observed value `current`.
    ///
    /// Wrap-correct: the distance is `target.wrapping_sub(current)` — a fence
    /// sequence crossing the 32-bit boundary arms and fires correctly. If the
    /// target is already reached at arm time (`current == target`), the arm fires
    /// immediately (returned as `Ok(Some(event))`, nothing is left armed).
    ///
    /// An identical re-send (same key, same target, same event) is idempotent; a
    /// *different* arm on a live key is a loud [`CompletionError::FenceRearm`];
    /// the table is capacity-bounded (boundary-1).
    pub fn arm(
        &mut self,
        key: (u64, u64),
        current: u32,
        target: u32,
        event: OsEventRef,
    ) -> Result<Option<OsEventRef>, CompletionError> {
        if let Some(existing) = self.armed.get(&key) {
            return if existing.target == target && existing.event == event {
                Ok(None) // retried arm: idempotent, the live arm stands
            } else {
                Err(CompletionError::FenceRearm { key })
            };
        }
        let to_go = u64::from(target.wrapping_sub(current));
        if to_go == 0 {
            return Ok(Some(event)); // already at target — complete now
        }
        if self.armed.len() >= MAX_ARMED_FENCES {
            return Err(CompletionError::FenceTableFull);
        }
        self.armed.insert(
            key,
            FenceArm {
                to_go,
                last: current,
                target,
                event,
            },
        );
        Ok(None)
    }

    /// A host write to fence `key` was observed carrying `value`.
    ///
    /// Applies the #12 jump guard first: a step further than [`MAX_FENCE_JUMP`]
    /// from the last accepted value (which is what a stale/backwards write looks
    /// like under wrap arithmetic) is a loud [`CompletionError::FenceJump`] and
    /// changes nothing. Otherwise the arm advances; reaching/passing the target
    /// fires it — the arm is consumed and its event returned exactly once.
    ///
    /// An observation on an un-armed key is `Ok(None)`: fence pages advance
    /// legitimately outside any armed window (pattern (a)-style traffic).
    pub fn observe(
        &mut self,
        key: (u64, u64),
        value: u32,
    ) -> Result<Option<OsEventRef>, CompletionError> {
        let Some(arm) = self.armed.get_mut(&key) else {
            return Ok(None);
        };
        let step = u64::from(value.wrapping_sub(arm.last));
        if step > MAX_FENCE_JUMP {
            return Err(CompletionError::FenceJump {
                last: arm.last,
                value,
            });
        }
        arm.last = value;
        arm.to_go = arm.to_go.saturating_sub(step);
        if arm.to_go == 0 {
            let fired = self.armed.remove(&key).expect("armed above").event;
            return Ok(Some(fired));
        }
        Ok(None)
    }
}

/// What the caller must do after a policy decision: post these events as one GSP
/// batch and raise one SWGEN0 edge (transport = `kayfabe-gsp`; irq = `Vmm::raise_irq`).
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

// The concurrency contract, compile-time-asserted (decision #17).
kayfabe_util::assert_send_sync!(
    OsEventRef,
    BatchId,
    CompletionError,
    CompletionQueue,
    DeliveryPlane,
    FenceArms,
    PostBatch,
);

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
        qb.observe(OsEventRef(0xb0)).unwrap();
        let post = plane
            .try_post([&mut qa, &mut qb])
            .expect("B's completion posts");
        assert_eq!(post.events, vec![OsEventRef(0xb0)]);
        plane.drained([&mut qa, &mut qb]);
        qb.ack(OsEventRef(0xb0));

        // A's completion is observed... but composed into a batch that the guest
        // drains WITHOUT A's waiter waking (the lost-wakeup window).
        qa.observe(OsEventRef(0xa0)).unwrap();
        let post = plane
            .try_post([&mut qa, &mut qb])
            .expect("A's completion posts");
        assert_eq!(post.events, vec![OsEventRef(0xa0)]);
        plane.drained([&mut qa, &mut qb]);
        // No ack from A. B rings no more doorbells — in the C, delivery is dead here.

        // A polls: its un-acked event is re-posted off its OWN poll.
        let repost = plane
            .on_poll(&mut qa, [&mut qb])
            .expect("poll re-posts A's event");
        assert_eq!(repost.events, vec![OsEventRef(0xa0)]);
        plane.drained([&mut qa, &mut qb]);
        qa.ack(OsEventRef(0xa0));
        assert!(!qa.has_outstanding());
        // And a poll with nothing outstanding posts nothing (no over-posting).
        assert!(plane.on_poll(&mut qa, [&mut qb]).is_none());
    }

    /// Pattern (e) semantics: below-target values do not fire; at/after-target
    /// fires exactly once (the arm is consumed); unrelated keys never fire.
    #[test]
    fn fence_arm_fires_at_or_after_target_exactly_once() {
        let mut f = FenceArms::new();
        let key = (0x0340_1000, 0x0002_0050_0000);
        assert_eq!(f.arm(key, 10, 14, OsEventRef(0xE)), Ok(None));
        assert_eq!(f.armed_len(), 1);

        // Below target: no fire. Unrelated key: no fire, no error.
        assert_eq!(f.observe(key, 12), Ok(None));
        assert_eq!(
            f.observe((0x9999, 0x1000), 14),
            Ok(None),
            "un-armed key is inert"
        );
        // Past target (skipped exactly-14 — "at/after"): fires, consumed.
        assert_eq!(f.observe(key, 15), Ok(Some(OsEventRef(0xE))));
        assert_eq!(f.armed_len(), 0);
        // Further advances are inert — the fence fired exactly once.
        assert_eq!(f.observe(key, 16), Ok(None));
    }

    /// The 32-bit wrap is handled: an arm spanning the u32 boundary fires when the
    /// wrapped value passes the wrapped target; arming AT the current value
    /// completes immediately.
    #[test]
    fn fence_arm_is_wrap_correct() {
        let mut f = FenceArms::new();
        let key = (1, 2);
        // current near u32::MAX, target wrapped past 0.
        assert_eq!(f.arm(key, u32::MAX - 2, 3, OsEventRef(0x77)), Ok(None));
        assert_eq!(f.observe(key, u32::MAX), Ok(None), "still before the wrap");
        assert_eq!(
            f.observe(key, 3),
            Ok(Some(OsEventRef(0x77))),
            "wrapped past target"
        );
        // Arm at-target: complete immediately, nothing left armed.
        assert_eq!(
            f.arm((5, 5), 42, 42, OsEventRef(0x88)),
            Ok(Some(OsEventRef(0x88)))
        );
        assert_eq!(f.armed_len(), 0);
    }

    /// ★ The #12 jump guard: a backwards (stale/foreign) write looks like an absurd
    /// forward step under wrap arithmetic and is REFUSED loudly — never counted as
    /// completion progress; the arm state is unchanged and still fires correctly.
    #[test]
    fn fence_backwards_jump_is_a_loud_refusal_never_a_completion() {
        let mut f = FenceArms::new();
        let key = (1, 2);
        f.arm(key, 100, 200, OsEventRef(0x9)).unwrap();
        // A stale value from before the arm: wrapping step ≈ 2^32 - 60 > MAX_FENCE_JUMP.
        assert_eq!(
            f.observe(key, 40),
            Err(CompletionError::FenceJump {
                last: 100,
                value: 40
            })
        );
        // The refusal changed nothing: the arm still fires at its target.
        assert_eq!(f.observe(key, 200), Ok(Some(OsEventRef(0x9))));
    }

    /// Retried-RPC discipline on the arm itself: an identical re-arm is idempotent;
    /// a different arm on the live key is a loud conflict.
    #[test]
    fn fence_rearm_idempotent_for_identical_loud_for_conflicting() {
        let mut f = FenceArms::new();
        let key = (1, 2);
        f.arm(key, 0, 10, OsEventRef(0x1)).unwrap();
        assert_eq!(
            f.arm(key, 0, 10, OsEventRef(0x1)),
            Ok(None),
            "identical re-send OK"
        );
        assert_eq!(f.armed_len(), 1, "no duplicate arm");
        assert_eq!(
            f.arm(key, 0, 99, OsEventRef(0x1)),
            Err(CompletionError::FenceRearm { key }),
            "conflicting re-arm is loud"
        );
    }

    /// The drain gate: one batch outstanding; composing is refused until drained
    /// (the seqNum-ring transport constraint) — but ONE batch may carry BOTH procs'
    /// independent completions (no cross-process serialization).
    #[test]
    fn drain_gate_batches_across_procs_without_serializing() {
        let mut plane = DeliveryPlane::new();
        let mut qa = CompletionQueue::new();
        let mut qb = CompletionQueue::new();
        qa.observe(OsEventRef(1)).unwrap();
        qb.observe(OsEventRef(2)).unwrap();
        let post = plane.try_post([&mut qa, &mut qb]).unwrap();
        assert_eq!(
            post.events,
            vec![OsEventRef(1), OsEventRef(2)],
            "one batch, both procs"
        );
        // Gate closed while outstanding.
        qa.observe(OsEventRef(3)).unwrap();
        assert!(
            plane.try_post([&mut qa, &mut qb]).is_none(),
            "gate closed until drain"
        );
        plane.drained([&mut qa, &mut qb]);
        let post = plane.try_post([&mut qa, &mut qb]).unwrap();
        assert_eq!(post.events, vec![OsEventRef(3)]);
    }

    /// ★ Mutation-gate kill (`outstanding_len` `+`→`*`, `has_outstanding` `||`→`&&`):
    /// the boundedness accounting must sum ALL THREE internal queues, and "has
    /// anything outstanding" must stay true whenever ANY one of them is non-empty —
    /// including a state where the OTHER two are empty. The prior suite only ever
    /// exercised states where at most one internal queue was non-empty at a time, so a
    /// `+`→`*` (0-absorbing) or a `||`→`&&` (all-required) mutation survived. This
    /// drives a queue through every co-populated combination and asserts the observable
    /// count + predicate directly.
    #[test]
    fn outstanding_accounting_sums_all_three_queues_and_predicate_is_any() {
        let mut q = CompletionQueue::new();
        // pending only.
        q.observe(OsEventRef(1)).unwrap();
        q.observe(OsEventRef(2)).unwrap();
        assert_eq!(q.outstanding_len(), 2);
        assert!(q.has_outstanding());

        // Compose one pending into in_flight, leave one pending: pending AND in_flight
        // both non-empty. `+` gives 2; `*` would give 1×1 = 1 — the kill.
        q.observe(OsEventRef(3)).unwrap(); // pending = {1,2,3}
        let posted = q.compose_into(BatchId(7)); // in_flight = {1,2,3}, pending = {}
        assert_eq!(posted.len(), 3);
        q.observe(OsEventRef(4)).unwrap(); // pending = {4}, in_flight = {1,2,3}
        assert_eq!(
            q.outstanding_len(),
            4,
            "pending(1) + in_flight(3) must SUM, not multiply"
        );
        assert!(q.has_outstanding());

        // Drain the batch: its events go to awaiting_ack; the freshly-observed 4 stays
        // pending. Now pending AND awaiting_ack both non-empty (in_flight empty).
        q.drained(BatchId(7)); // awaiting_ack = {1,2,3}, pending = {4}
        assert_eq!(
            q.outstanding_len(),
            4,
            "pending(1) + awaiting_ack(3) must SUM even with in_flight empty",
        );
        assert!(q.has_outstanding());

        // Ack the lone pending event: ONLY awaiting_ack remains non-empty. The
        // `has_outstanding` `||`→`&&` mutant would report FALSE here (pending empty);
        // the truth is TRUE — un-acked completions are still outstanding.
        q.ack(OsEventRef(4));
        assert_eq!(q.outstanding_len(), 3, "only awaiting_ack remains");
        assert!(
            q.has_outstanding(),
            "un-acked (drained-but-unconsumed) completions ARE outstanding, even alone",
        );

        // Ack all three awaiting-ack events → fully drained → nothing outstanding.
        q.ack(OsEventRef(1));
        q.ack(OsEventRef(2));
        q.ack(OsEventRef(3));
        assert_eq!(q.outstanding_len(), 0);
        assert!(!q.has_outstanding());
    }

    /// ★ Mutation-gate kill (`ack` in_flight retain `!=`→`==`): `ack` must remove the
    /// consumed event from EVERY internal queue, including one still `in_flight`
    /// (composed into a posted-but-not-yet-drained batch). The prior suite only acked
    /// events that had already reached `awaiting_ack`, so the in_flight retain arm was
    /// never exercised — a `!=`→`==` there (which would DELETE every OTHER in-flight
    /// event and KEEP the acked one) survived. Here the guest acks an in-flight event
    /// directly (its waiter woke before the IRQSCLR drain); the acked event must be
    /// gone and its in-flight sibling must remain.
    #[test]
    fn ack_removes_an_in_flight_event_and_spares_its_siblings() {
        let mut q = CompletionQueue::new();
        q.observe(OsEventRef(0xA)).unwrap();
        q.observe(OsEventRef(0xB)).unwrap();
        let posted = q.compose_into(BatchId(1)); // both now in_flight, pending empty
        assert_eq!(posted, vec![OsEventRef(0xA), OsEventRef(0xB)]);
        assert_eq!(q.outstanding_len(), 2);

        // Ack 0xA while it is still in_flight (waiter woke pre-drain).
        q.ack(OsEventRef(0xA));
        assert_eq!(
            q.outstanding_len(),
            1,
            "exactly the acked in-flight event is removed"
        );
        assert!(q.has_outstanding(), "0xB is still in flight");

        // Draining the batch must carry 0xB (the survivor) to awaiting_ack — and NOT
        // resurrect 0xA. The `==` mutant would have kept 0xA and dropped 0xB here.
        q.drained(BatchId(1));
        assert_eq!(q.take_unacked(), 1, "only the un-acked 0xB awaits re-post");
        q.ack(OsEventRef(0xB));
        assert!(!q.has_outstanding());
    }

    /// ★ Mutation-gate kill (`MAX_FENCE_JUMP` `2 * 1024`→`2 + 1024`): the #12 jump
    /// guard's bound is observable, not a free-to-tune constant — a LEGITIMATE fence
    /// step just under `2 * 1024 = 2048` must be ACCEPTED (a real GPFIFO can advance a
    /// completion sema by up to 2× its entry count in one observation), while an absurd
    /// backwards/wrap step is refused. The `2 + 1024 = 1026` mutant lowers the cap so a
    /// legitimate 1500-step is wrongly rejected as a jump — this test pins the real
    /// boundary: 1500 accepted, 2^32-scale refused.
    #[test]
    fn fence_jump_guard_accepts_a_legitimate_large_step() {
        assert_eq!(MAX_FENCE_JUMP, 2048, "the guard is 2 x max GPFIFO entries");
        let mut f = FenceArms::new();
        let key = (7, 7);
        // Target far ahead so the legitimate step below does not itself complete.
        f.arm(key, 0, 100_000, OsEventRef(0xF)).unwrap();
        // A 1500-step advance is legitimate (< 2048) — it must be accepted, not faulted.
        // Under the `2 + 1024 = 1026` mutant this loud-faults as a spurious jump.
        assert_eq!(
            f.observe(key, 1500),
            Ok(None),
            "a <2048 step is legitimate progress"
        );
        // A step of EXACTLY MAX_FENCE_JUMP (1500 -> 1500+2048 = 3548) is at the bound and
        // must be ACCEPTED (`step > MAX_FENCE_JUMP` is false at equality). The `>`→`>=`
        // mutant rejects this exactly-at-bound step — this is its kill.
        assert_eq!(
            f.observe(key, 1500 + MAX_FENCE_JUMP as u32),
            Ok(None),
            "a step of exactly MAX_FENCE_JUMP is at the bound and accepted",
        );
        // An absurd backwards write (≈2^32 under wrap) is still refused loudly.
        let last = 1500 + MAX_FENCE_JUMP as u32;
        assert_eq!(
            f.observe(key, 1),
            Err(CompletionError::FenceJump { last, value: 1 }),
            "a wrap-scale backwards step is a loud refusal",
        );
    }

    /// ★ Mutation-gate kill (`DeliveryPlane::batch_outstanding`→`true`): the drain-gate
    /// state predicate must report FALSE on a fresh plane and again after a drain — it
    /// is what a caller checks before posting (over-posting desyncs the seqNum ring,
    /// L10). The prior suite drove `try_post`/`drained` but never asserted
    /// `batch_outstanding` directly, so a "always true" mutant survived. This pins the
    /// observable gate state across the full post→drain cycle.
    #[test]
    fn batch_outstanding_tracks_the_drain_gate_state() {
        let mut plane = DeliveryPlane::new();
        let mut q = CompletionQueue::new();
        // Fresh plane: nothing posted, gate OPEN.
        assert!(
            !plane.batch_outstanding(),
            "a fresh plane has no batch outstanding"
        );

        q.observe(OsEventRef(1)).unwrap();
        plane.try_post([&mut q]).expect("posts the pending event");
        // A batch is now outstanding: gate CLOSED.
        assert!(
            plane.batch_outstanding(),
            "after a post a batch is outstanding"
        );

        plane.drained([&mut q]);
        // Drained: gate OPEN again.
        assert!(
            !plane.batch_outstanding(),
            "after the drain the gate is open again"
        );
    }

    /// ★ Mutation-gate kill (`DeliveryPlane::try_post` `next_batch += 1`→`*= 1`): each
    /// posted batch must get a DISTINCT, monotonic `BatchId` — the id is the drain key
    /// (`CompletionQueue::drained(batch)` moves exactly that batch's in-flight events),
    /// so a reused id would let one drain sweep a later batch's events. `next_batch`
    /// starts at 0, so `*= 1` pins it at 0 forever (every batch is `BatchId(0)`), while
    /// `+= 1` increments. The prior suite never asserted the id VALUE across two posts.
    #[test]
    fn successive_batches_carry_distinct_monotonic_ids() {
        let mut plane = DeliveryPlane::new();
        let mut q = CompletionQueue::new();

        q.observe(OsEventRef(1)).unwrap();
        let first = plane.try_post([&mut q]).expect("first post");
        plane.drained([&mut q]);

        q.observe(OsEventRef(2)).unwrap();
        let second = plane.try_post([&mut q]).expect("second post");

        assert_ne!(
            first.batch, second.batch,
            "successive batches must carry distinct ids (the drain key must not collide)",
        );
        assert!(
            second.batch.0 > first.batch.0,
            "batch ids advance monotonically"
        );
    }
}
