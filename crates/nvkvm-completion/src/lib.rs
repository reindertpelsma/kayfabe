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
//!
//! Concurrency (decision #17): plain owned data, no interior mutability; all
//! mutation is `&mut self` — see `nvkvm-core`'s crate docs for the full contract.
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
        self.armed.insert(key, FenceArm { to_go, last: current, target, event });
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
            return Err(CompletionError::FenceJump { last: arm.last, value });
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

// The concurrency contract, compile-time-asserted (decision #17).
nvkvm_util::assert_send_sync!(
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
        let post = plane.try_post([&mut qa, &mut qb]).expect("B's completion posts");
        assert_eq!(post.events, vec![OsEventRef(0xb0)]);
        plane.drained([&mut qa, &mut qb]);
        qb.ack(OsEventRef(0xb0));

        // A's completion is observed... but composed into a batch that the guest
        // drains WITHOUT A's waiter waking (the lost-wakeup window).
        qa.observe(OsEventRef(0xa0)).unwrap();
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
        assert_eq!(f.observe((0x9999, 0x1000), 14), Ok(None), "un-armed key is inert");
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
        assert_eq!(f.observe(key, 3), Ok(Some(OsEventRef(0x77))), "wrapped past target");
        // Arm at-target: complete immediately, nothing left armed.
        assert_eq!(f.arm((5, 5), 42, 42, OsEventRef(0x88)), Ok(Some(OsEventRef(0x88))));
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
            Err(CompletionError::FenceJump { last: 100, value: 40 })
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
        assert_eq!(f.arm(key, 0, 10, OsEventRef(0x1)), Ok(None), "identical re-send OK");
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
        assert_eq!(post.events, vec![OsEventRef(1), OsEventRef(2)], "one batch, both procs");
        // Gate closed while outstanding.
        qa.observe(OsEventRef(3)).unwrap();
        assert!(plane.try_post([&mut qa, &mut qb]).is_none(), "gate closed until drain");
        plane.drained([&mut qa, &mut qb]);
        let post = plane.try_post([&mut qa, &mut qb]).unwrap();
        assert_eq!(post.events, vec![OsEventRef(3)]);
    }
}
