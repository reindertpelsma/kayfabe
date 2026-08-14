//! ★★★★★ **w323 — THE DEFERRED PUBLICATION LANE: what a doorbell trap may hand to a
//! worker, and what it may never.**
//!
//! Owner ruling, 2026-08-14: *"10 ms BQL lock seems already bad to me. … why do you need to
//! publish under BQL? I don't see a reason. you already have other boundaries to determine
//! when to update a pte write — look at the C."*
//!
//! Full argument: `docs/design/publication_off_the_bql.md`. This module is the mechanism
//! only, and it is deliberately free of every GPU concept except the guest's opaque
//! doorbell token, so its properties can be tested with no device, no isolate and no boot.
//!
//! # 1. Why deferral is legal at all — the ordering requirement is NOT what it looks like
//!
//! The requirement is **publish → *our* host ring**. ⊘ It is **not** *publish → the guest's
//! MMIO doorbell store*, and conflating those two is what made this look impossible for
//! three rungs. The guest's store is **fire-and-forget by the GPFIFO contract**: the guest
//! does not read a result, does not poll a status and cannot observe when we act. Both ends
//! of the real ordering constraint are **ours**.
//!
//! ⇒ we may return from the trap immediately, publish on a worker, and ring the host when
//! publication is complete. The guest cannot tell the difference, because it never waited.
//!
//! ★ The C did exactly this and its own source says so
//! (`C: src/qemu/nvkvm_gpu_emul.c:592-604`): an **O(1) dirty latch at the write**, then a
//! **deferred commit at the release semaphore** — *"before the release un-gates the
//! (already-rung, host-resident) GR channel into the new mapping"*. **The channel was
//! already rung.**
//!
//! # 2. ★★★ WHY THERE IS NO OVERFLOW PROBLEM — a doorbell is a LEVEL, not an EDGE
//!
//! The obvious design is a per-channel FIFO of pending doorbells, and then the brief's
//! question *"what happens on overflow"* is real and unpleasant. It dissolves:
//!
//! **The submission cursor is read at EXECUTION time, not latched at trap time.**
//! `SharedDevice::doorbell` → `forward_ring` reads the channel's GPFIFO `GP_PUT` out of
//! guest memory when it runs. So *N* doorbells on one token and *one* doorbell after the
//! last `GP_PUT` advance are **the same act**: both execute against the newest cursor.
//!
//! ⇒ the queue holds **at most one entry per distinct token**, and re-offering a token
//! already pending is a coalesce, not a push. Its size is bounded by the number of live
//! channels, which is a function of the *guest's allocations* — a number RM already bounds
//! and which no amount of doorbell ringing can inflate. **A guest cannot make this grow by
//! ringing faster**, which is the property an unbounded queue on a guest-driven path
//! actually needs.
//!
//! ⚠ **The one thing coalescing costs, stated rather than discovered later:** the worker
//! reads the ring while the guest is running, where today it reads it with the guest
//! halted. A guest that rewrites a GPFIFO entry between advancing `GP_PUT` and the engine
//! consuming it now races us. ⊘ That is **already illegal for the guest against real
//! hardware** — the GPU DMA-reads the pushbuffer asynchronously and the GPFIFO contract
//! forbids exactly this — so the deferral is *more* faithful, not less. What it is not is
//! *identical*: our decoder is not the hardware's, and a torn read must fault rather than
//! be acted on. That obligation is the ring reader's, not this queue's, and it is named in
//! `publication_off_the_bql.md` §5.3.
//!
//! # 3. ⊘⊘⊘ THE ASYMMETRY THAT IS THE WHOLE SAFETY ARGUMENT — MAP MAY DEFER, REVOKE MAY NOT
//!
//! Owner ruling, folded in above the mechanism because the mechanism enforces it:
//!
//! | direction | what a late act costs |
//! |---|---|
//! | **MAP** (invalid → valid) | the engine walks an unpublished VA ⇒ a **GPU fault**. **FAIL-SAFE**: this tree measured containment — a bystander ran 2 675 519 verified iterations through one. |
//! | **REVOKE** (valid → invalid) | a **live host-GPU translation into guest pages the guest has already released and Linux has reused**. **FAIL-DANGEROUS.** |
//!
//! ⇒ ★★★★★ **DEFERRING A REVOCATION IS NOT A LATENCY CHOICE, IT IS A LEAK WINDOW** whose
//! duration is exactly the deferral. This module makes that **not expressible**: the queue's
//! item type is [`MapPublication`] and there is no `From`, no `Deref` and no constructor
//! that turns a revocation into one. A caller that tries hands a [`Revocation`] to a
//! parameter that does not accept it and **does not compile**
//! (`tests/ui/defer_a_revocation.rs`).
//!
//! ⊘ **And a guest omission must not be able to open the window either.** The revocation
//! lane is driven by *our* teardown of *our* host objects — `Proc::stage_release` and the
//! budgeted drain — never by a guest signal we could be denied. Withholding whatever would
//! have triggered a publish cannot extend a revocation's window, because no guest action is
//! on that path at all.
//!
//! # 4. ⊘ AND THE OTHER INVALIDATE — the one that is OURS, and which is already discharged
//!
//! Two unrelated things wear the word:
//! 1. **the GUEST's** TLB invalidate, which we might observe as a publish trigger. ⊘ On our
//!    Mode-2 compute path the guest issues **zero** — measured, `mode2_address_table.md` §5
//!    audit S3 — so it cannot be a commit boundary and nothing here waits for one;
//! 2. **the invalidate WE owe the HOST GPU** after a host PTE changes.
//!
//! ★ (2) is **discharged by the host RM, by construction, on every path we own**: we never
//! author a host PTE. Every host mapping change of ours is an RM ioctl
//! (`NV_ESC_RM_MAP_MEMORY_DMA` / `NV_ESC_RM_UNMAP_MEMORY_DMA`), and RM invalidates inside
//! it — `dmaUnmapBuffer` → `vaspaceInvalidateTlb(pVAS, pGpu, PTE_DOWNGRADE)`
//! (`ogkm: src/nvidia/src/kernel/gpu/mem_mgr/dma.c:899`, again `:1010`), `PTE_UPGRADE` on
//! the map side (`.../arch/maxwell/virt_mem_allocator_gm107.c:3032`).
//!
//! ⚠ **The consequence for this design, and it is the reason the asymmetry above is not
//! merely prudent:** RM's invalidate happens **inside the ioctl we are deferring**. Deferring
//! a revocation therefore defers its TLB invalidate by exactly the same interval. The leak
//! window is not an artefact of our bookkeeping; it is a real interval during which the host
//! GMMU still holds the translation.

use std::collections::{HashSet, VecDeque};
use std::sync::{Condvar, Mutex};

/// ★★★ **A publication that MAY be deferred** — the map direction, and the only thing
/// [`PublicationQueue`] accepts.
///
/// The field is private and the only constructor is [`MapPublication::for_doorbell`], so
/// the type's guarantee — *"this is an invalid→valid transition, whose late arrival costs a
/// GPU fault and nothing worse"* — is a claim about how the value was **obtained**, which
/// only a constructor can make.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MapPublication {
    token: u64,
}

impl MapPublication {
    /// The guest rang `token`; the mappings its submission needs are to be published and
    /// the host channel rung, on a worker.
    #[must_use]
    pub const fn for_doorbell(token: u64) -> Self {
        Self { token }
    }

    /// The guest's own doorbell token.
    #[must_use]
    pub const fn token(self) -> u64 {
        self.token
    }
}

/// ⊘⊘ **A revocation — valid→invalid. It exists HERE so that it can be REFUSED here.**
///
/// It carries no data and is deliberately useless: its entire purpose is to be a distinct
/// Rust type from [`MapPublication`], so *"put the unmap on the deferred queue"* is a type
/// error rather than a code review that has to happen every time.
///
/// ⊘ There is **no** `From<Revocation> for MapPublication`, no `Deref`, no `as_map()`, and
/// no public field — the three dismantling routes `host_class_role_wiring.rs` enumerates
/// for a role type, closed by construction and pinned by
/// `tests/ui/defer_a_revocation.rs` + `the_revocation_type_has_no_route_into_the_queue`.
///
/// ★ Where revocations actually go: `Proc::stage_release` → the **budgeted, synchronous**
/// drain in `Regs::write`. That is tier 2 — bounded, inline, and correct precisely because
/// it is bounded (`blocking_and_completion_model.md` §2). It is **not** free and it is not
/// pretended to be; it is a far smaller BQL cost than today's whole-VAS publication drain,
/// and it is the honest floor. See `publication_off_the_bql.md` §4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Revocation {
    /// ⊘ Private and unused: the type is a *refusal*, not a payload. A `pub` field would be
    /// the "uniform escape" route a role type must not have.
    _unused: (),
}

impl Revocation {
    /// Name a revocation. ⊘ It cannot be enqueued; see the type's docs.
    #[must_use]
    pub const fn of_a_host_mapping() -> Self {
        Self { _unused: () }
    }
}

/// What [`PublicationQueue::offer`] did with a token — the two outcomes are different
/// facts and a `bool` would have hidden which.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offered {
    /// A new entry joined the queue.
    Queued,
    /// ★ This token was **already pending**, so the offer coalesced into it. Not a drop:
    /// the pending execution will read the newest cursor and therefore covers this
    /// doorbell too. See the module docs §2.
    Coalesced,
    /// ⊘ **REFUSED** — the queue is at its distinct-token cap. The caller must fall back to
    /// running the work inline, which is today's behaviour and is therefore never worse
    /// than the status quo. Counted, never silent.
    Full,
}

/// Everything the queue can say about itself, as one value, so a log line cannot report a
/// subset and a reader cannot infer the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QueueStats {
    /// Offers that created a new entry.
    pub queued: u64,
    /// Offers that folded into a pending entry (§2 — a doorbell is a level).
    pub coalesced: u64,
    /// ⊘ Offers refused at the cap. **Non-zero means some doorbell ran inline**, and the
    /// caller owes a line saying so.
    pub refused: u64,
    /// Entries the worker took.
    pub taken: u64,
    /// The deepest the queue has ever been — the bound, measured rather than assumed.
    pub high_water: usize,
}

/// The shared state, behind one leaf mutex.
#[derive(Debug, Default)]
struct Inner {
    /// FIFO of tokens awaiting execution. ★ Order across tokens is preserved; order
    /// *within* a token is vacuous because there is at most one entry per token.
    order: VecDeque<u64>,
    /// Membership, so `offer` is O(1) rather than a scan of `order`.
    pending: HashSet<u64>,
    stats: QueueStats,
    /// Set once; wakes the worker so it can exit rather than park forever.
    stopping: bool,
    /// ★ Monotonic count of completed executions, so a test (or a wait-for-quiesce) can ask
    /// *"has everything offered before instant T been acted on?"* without polling the
    /// queue's emptiness — which is true both before the worker starts and after it
    /// finishes, and therefore answers nothing on its own.
    completed: u64,
}

/// ★★★ **The lane.** Single consumer, many producers, coalescing, capped.
///
/// ⊘ It holds **no reference to the device, the plane or any isolate** — deliberately, and
/// it is the same law `kayfabe_rt::inbox` states: *"a producer that wanted to touch core
/// state has nothing to touch"*. Everything it carries is a `u64` the guest wrote.
#[derive(Debug)]
pub struct PublicationQueue {
    inner: Mutex<Inner>,
    wake: Condvar,
    cap: usize,
}

impl PublicationQueue {
    /// ★★ **The distinct-token cap.**
    ///
    /// Not a rate limit — §2 explains why a rate cannot grow this queue. It is a bound on
    /// **live channels with work outstanding**, and it is set well above anything RM
    /// permits a single guest to hold open so that hitting it is a *diagnosis*, not a
    /// routine. On overflow the caller runs inline, i.e. degrades to today's behaviour.
    pub const DEFAULT_CAP: usize = 4096;

    /// A queue with [`Self::DEFAULT_CAP`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_cap(Self::DEFAULT_CAP)
    }

    /// A queue with an explicit cap — for tests that want to reach the refusal arm without
    /// building 4096 channels. ⊘ A cap of 0 is legal and refuses everything, which is the
    /// known-positive the refusal arm needs.
    #[must_use]
    pub fn with_cap(cap: usize) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            wake: Condvar::new(),
            cap,
        }
    }

    /// ★★★★★ **THE BQL-SIDE CALL, and the whole point: it does no host I/O.**
    ///
    /// A hash insert, a `VecDeque` push and a `notify_one`, under one leaf mutex that no
    /// other trap path takes for anything but this. That is `INLINE-SAFE` clause (a) (it
    /// completes without the guest running), clause (b) (it is O(1)) and clause (c) (the
    /// lock is a leaf held for a handful of instructions).
    ///
    /// ⊘ It takes a [`MapPublication`], not a token — so *"defer the unmap"* is a type
    /// error at the call site rather than a review comment. See the module docs §3.
    pub fn offer(&self, job: MapPublication) -> Offered {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let token = job.token();
        if g.pending.contains(&token) {
            g.stats.coalesced += 1;
            return Offered::Coalesced;
        }
        if g.order.len() >= self.cap {
            g.stats.refused += 1;
            return Offered::Full;
        }
        g.pending.insert(token);
        g.order.push_back(token);
        g.stats.queued += 1;
        g.stats.high_water = g.stats.high_water.max(g.order.len());
        drop(g);
        self.wake.notify_one();
        Offered::Queued
    }

    /// Take the next token, or `None` if the queue is empty. Non-blocking.
    ///
    /// ⊘ The token leaves `pending` **here**, at the take — not after execution. That is
    /// deliberate and it is the safe direction: a doorbell offered *while* this token is
    /// executing must queue again, because the guest may have advanced `GP_PUT` after the
    /// worker read it. Clearing on completion instead would coalesce that doorbell into a
    /// run that had already looked, and the submission would sit unexecuted until the next
    /// unrelated doorbell — a **lost wakeup**, which is the classic bug of this shape.
    pub fn take(&self) -> Option<MapPublication> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let token = g.order.pop_front()?;
        g.pending.remove(&token);
        g.stats.taken += 1;
        Some(MapPublication::for_doorbell(token))
    }

    /// Block until there is a token or the queue is stopping. `None` ⇒ stop.
    ///
    /// ⊘ Called **only** from the worker thread. A vCPU that called this would be waiting
    /// on something the guest must do, under the BQL — clause (a)'s guaranteed deadlock —
    /// which is why the worker is the only thing this crate hands the queue to in a
    /// blocking role.
    pub fn take_blocking(&self) -> Option<MapPublication> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(token) = g.order.pop_front() {
                g.pending.remove(&token);
                g.stats.taken += 1;
                return Some(MapPublication::for_doorbell(token));
            }
            if g.stopping {
                return None;
            }
            g = self.wake.wait(g).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// The worker announces one execution finished. Bumps [`Self::completed`].
    pub fn note_completed(&self) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.completed += 1;
        drop(g);
        self.wake.notify_all();
    }

    /// How many executions the worker has finished. Monotonic.
    #[must_use]
    pub fn completed(&self) -> u64 {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .completed
    }

    /// How many tokens are waiting right now.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .order
            .len()
    }

    /// This queue's numbers.
    #[must_use]
    pub fn stats(&self) -> QueueStats {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stats
    }

    /// Tell the worker to finish the queue and exit.
    pub fn stop(&self) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.stopping = true;
        drop(g);
        self.wake.notify_all();
    }

    /// One line for a boot log. ⊘ Prints `refused` even when it is 0 — a publication lane
    /// that never refused and one that was never armed are different facts, and only the
    /// presence of the line distinguishes them.
    #[must_use]
    pub fn census(&self) -> String {
        let s = self.stats();
        format!(
            "PUBQUEUE queued={} coalesced={} refused={} taken={} completed={} depth={} \
             high_water={} cap={}{}",
            s.queued,
            s.coalesced,
            s.refused,
            s.taken,
            self.completed(),
            self.depth(),
            s.high_water,
            self.cap,
            if s.refused > 0 {
                " ⚠⚠ REFUSED>0 — those doorbells ran INLINE under the BQL"
            } else {
                ""
            },
        )
    }
}

impl Default for PublicationQueue {
    fn default() -> Self {
        Self::new()
    }
}

kayfabe_util::assert_send_sync!(PublicationQueue, MapPublication, Revocation);

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// ★★★ **THE COALESCE, which is what makes the overflow question dissolve.** Ten
    /// doorbells on one token occupy one slot, and the queue says `Coalesced` rather than
    /// pretending it queued them.
    #[test]
    fn many_doorbells_on_one_token_occupy_one_slot() {
        let q = PublicationQueue::new();
        assert_eq!(q.offer(MapPublication::for_doorbell(7)), Offered::Queued);
        for _ in 0..9 {
            assert_eq!(q.offer(MapPublication::for_doorbell(7)), Offered::Coalesced);
        }
        assert_eq!(q.depth(), 1);
        let s = q.stats();
        assert_eq!((s.queued, s.coalesced, s.refused), (1, 9, 0));
    }

    /// …and distinct tokens do **not** coalesce, which is the control the test above needs:
    /// a queue that coalesced everything would pass it and serve one channel.
    #[test]
    fn distinct_tokens_each_get_their_own_slot() {
        let q = PublicationQueue::new();
        for t in 0..8 {
            assert_eq!(q.offer(MapPublication::for_doorbell(t)), Offered::Queued);
        }
        assert_eq!(q.depth(), 8);
        assert_eq!(q.stats().high_water, 8);
    }

    /// ★★ **ORDER ACROSS TOKENS IS FIFO** — the property `publication_off_the_bql.md` §3
    /// relies on for cross-channel fairness. (Order *within* a token is vacuous: there is
    /// never more than one entry.)
    #[test]
    fn tokens_come_back_in_the_order_they_were_offered() {
        let q = PublicationQueue::new();
        for t in [40, 10, 30, 20] {
            q.offer(MapPublication::for_doorbell(t));
        }
        let got: Vec<u64> = std::iter::from_fn(|| q.take()).map(|j| j.token()).collect();
        assert_eq!(got, vec![40, 10, 30, 20]);
    }

    /// ★★★★★ **THE LOST-WAKEUP PROPERTY, and it is the one a naive implementation gets
    /// wrong.** A doorbell that arrives *after* its token was taken must queue again, not
    /// coalesce into the run that has already read the cursor.
    #[test]
    fn a_doorbell_arriving_after_the_take_queues_again() {
        let q = PublicationQueue::new();
        q.offer(MapPublication::for_doorbell(5));
        let job = q.take().expect("one job");
        assert_eq!(job.token(), 5);
        // The worker is now "executing" token 5; the guest advances GP_PUT and rings again.
        assert_eq!(
            q.offer(MapPublication::for_doorbell(5)),
            Offered::Queued,
            "coalescing here would lose the submission until an unrelated doorbell arrived"
        );
        assert_eq!(q.depth(), 1);
    }

    /// ⊘ **The refusal arm, with a known-positive** — a cap of 0 refuses, and the census
    /// says out loud that those doorbells ran inline. A queue whose refusal path is never
    /// exercised is a fallback nobody has ever seen work.
    #[test]
    fn a_full_queue_refuses_by_name_and_the_census_says_it_ran_inline() {
        let q = PublicationQueue::with_cap(1);
        assert_eq!(q.offer(MapPublication::for_doorbell(1)), Offered::Queued);
        assert_eq!(q.offer(MapPublication::for_doorbell(2)), Offered::Full);
        assert_eq!(q.stats().refused, 1);
        assert!(q.census().contains("ran INLINE under the BQL"), "{}", q.census());
        // …and the control: below the cap, no refusal and no warning.
        let ok = PublicationQueue::with_cap(4);
        ok.offer(MapPublication::for_doorbell(1));
        assert!(!ok.census().contains("REFUSED>0"));
    }

    /// ★★ **The BQL-side call takes no lock any other trap path takes and does no I/O** —
    /// asserted the only way a unit test can: it completes with no worker running at all,
    /// so nothing it does can be waiting on one.
    #[test]
    fn the_offer_completes_with_no_consumer_in_existence() {
        let q = PublicationQueue::new();
        for t in 0..1000 {
            q.offer(MapPublication::for_doorbell(t));
        }
        assert_eq!(q.depth(), 1000);
        assert_eq!(q.completed(), 0, "nothing has executed, and the offer did not wait");
    }

    /// The worker end: blocks, wakes on an offer, and exits on `stop` rather than parking
    /// forever.
    #[test]
    fn the_worker_wakes_on_an_offer_and_exits_on_stop() {
        let q = Arc::new(PublicationQueue::new());
        let w = {
            let q = Arc::clone(&q);
            std::thread::spawn(move || {
                let mut seen = Vec::new();
                while let Some(job) = q.take_blocking() {
                    seen.push(job.token());
                    q.note_completed();
                }
                seen
            })
        };
        for t in 0..16 {
            q.offer(MapPublication::for_doorbell(t));
        }
        // Wait for the worker to drain, then stop it.
        while q.completed() < 1 {
            std::thread::yield_now();
        }
        q.stop();
        let seen = w.join().expect("worker joins");
        assert!(!seen.is_empty(), "the worker must have executed something");
        assert_eq!(q.depth(), 0, "stop must drain, not abandon");
    }

    /// ⊘⊘ **THE ASYMMETRY, asserted the only way a runtime test can — by showing the type
    /// carries no route.** The compile-time half is `tests/ui/defer_a_revocation.rs`; this
    /// is the census half, and the two are complements (a type guards the boundary, a
    /// census guards the mint).
    #[test]
    fn the_revocation_type_has_no_route_into_the_queue() {
        let src = include_str!("pubqueue.rs");
        // Strip the test module, so this file's own prose about the absence does not read
        // as the presence.
        let body = src.split("#[cfg(test)]").next().expect("a non-test half");
        for route in [
            "impl From<Revocation>",
            "fn as_map",
            "impl std::ops::Deref for Revocation",
            "pub token:",
        ] {
            assert!(
                !body.contains(route),
                "a uniform escape off the revocation/map split appeared: {route}"
            );
        }
        // …and the queue's own signature still names the map type, so the split is asked
        // for at the point of use rather than untagged there.
        assert!(body.contains("pub fn offer(&self, job: MapPublication)"));
    }
}
