//! ★★ **The lock step, EXHAUSTIVELY** (`docs/design/V3_DISPLAY_PACING_20261017.md` §11.7, form A;
//! owner decisions §AK/§AL, 2026-10-17) — a bounded model checker over the REAL pure components, in the
//! style of `kf-chan/tests/user_loop_model.rs`. GPU-free, deterministic, no wall-clock time anywhere:
//! time is a number the explorer advances, and every internal deadline (the free clock, the broker's
//! silence, the credit gate, the copy, the relay's backstop and silence) fires exactly at its time.
//!
//! WHAT RUNS FOR REAL (the code under test, driven by the explorer): [`crate::tick::TickSource`] through
//! [`crate::pace::Pacer`] (guard, silence, handoffs, flap damping), [`crate::lockstep::CreditGate`],
//! [`crate::pace::Mailbox`], [`crate::pace::flip_completion_need`], and `kf_broker::ticks`
//! ([`Requests`], [`frame_gate`], [`tick_live`], the constants). WHAT IS MODELLED (the glue that lives in
//! I/O code and cannot be cloned): the relay's `progress`/`commit`/`release`/backstop/reclaim logic
//! (`conn.rs`), the lock-free tick share, the display worker's pass (`display.rs`), the ring's slots and
//! the guest's flip loop. A mutant in `conn.rs`/`display.rs` is therefore NOT seen here (the harness of
//! form B runs those); a mutant in the pure components is.
//!
//! THE STEPS ("letters", each applied at the current instant after an optional advance of time, each
//! followed by the worker's pass): guest flips (immediate, vsync); broker ticks (fresh with/without
//! `WANT_FRAME`, skipping, duplicate, stale), bursts of 10/100/1000 in one instant, sustained floods at
//! 10x/100x/1000x the refresh; the broker releasing a frame, the legacy `EV_FRAME`; reconnects; flapping
//! patterns; and time advances to every boundary (copy end, guard edge +-1 ns, silence edge +-1 ns, 49/50/51
//! ms, 200 ms, 2 s, 10 s). Exploration: depth-first with a visited set over the world's behaviour-relevant
//! state (times taken relative to `now`).
//!
//! INVARIANTS, checked after EVERY step (the list is the report's):
//!  1. no immediate flip is parked: it latches at the step it arrives and never waits for a credit;
//!  2. the completion queue never holds an item whose barrier is done, and no item waits for a copy that
//!     has not started (a flip never waits for a credit or a tick);
//!  3. no frame is sent without a credit: while ticks are live every send spends exactly one open request
//!     (checked independently of `frame_gate`), a request is spent once, the label is the newest request;
//!  4. one copy at a time, and only on a held credit; a credit is granted to the mailbox only at its
//!     tick's stamp or its guard edge, never earlier; granted <= offered;
//!  5. frames in flight <= 2 (the literal), a free ring slot always exists for a copy, a held slot is
//!     never overwritten;
//!  6. no double tick closer than the guard; the vblank number advances by exactly one; stamps never go
//!     back; the vblank never stalls for more than the silence + one period + the guard;
//!  7. sequence numbers strictly increase at the relay (wrapping);
//!  8. the buffer-reuse rule: no completion releases a buffer a copy in flight is reading;
//!  9. source changes: a hand back to the broker after a fallback needs a proving event first;
//! 10. counters are exact: ticks offered = accepted + stale; wants = frames + NO_CHANGE + replaced +
//!     open; gate wants = granted + replaced + stale + pending + dropped by a reset;
//! 11. `NO_CHANGE` only at a later tick than the one that opened the request, and not sooner than the
//!     answer gap after the last answer.
#![allow(clippy::too_many_lines, clippy::needless_range_loop)]

use super::HeadMode;
use crate::lockstep::CreditGate;
use crate::pace::{Mailbox, Pacer, flip_completion_need};
use crate::tick::{BROKER_SILENCE_NS, GUARD_DEN, GUARD_NUM, Handoff, Source, seq_newer};
use kf_broker::slots::HELD_CAP;
use kf_broker::ticks::{BROKER_SILENCE_MS, FrameGate, Requests, TickVerdict, frame_gate, tick_live};
use kf_broker::wire::TickPkt;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};

const MS: u64 = 1_000_000;
/// The guest mode's period: a whole number of milliseconds so the relay's ms clock and the worker's ns
/// clock stay in step (the +-1 ns letters probe the sub-ms edges).
const P: u64 = 16 * MS;
const GUARD: u64 = P / GUARD_DEN * GUARD_NUM;
/// One scanout copy.
const COPY: u64 = 3 * MS;
/// The start of time (never 0: nothing is "before" it).
const T0: u64 = 1_000 * MS;
/// The relay's legacy backstop and reclaim (conn.rs `CREDIT_BACKSTOP_MS`, `RECLAIM_AFTER_MS`).
const BACKSTOP_MS: u64 = 100;
const RECLAIM_MS: u64 = 1000;
/// The longest the guest's vblank may stall: the silence, one period and the guard (a swallowed event
/// still proves the broker alive, §5). The strict "50 ms + one period" is exceeded by up to the guard:
/// [`lockstep_exhaustive_the_strict_stall_bound_has_a_counterexample`].
const STALL_BOUND: u64 = BROKER_SILENCE_NS + P + GUARD;
const SLOTS: usize = 5;

#[derive(Clone, Copy, Debug)]
struct Cfg {
    /// The tick number the broker starts at (near `u32::MAX` to cross the wrap).
    start_seq: u32,
    /// The worker resets its gate when the connection changes (the design; the code does not yet).
    reset_gate: bool,
    /// A deliberately broken world, for the explorer's own test.
    bug: Option<Bug>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Bug {
    /// The relay sends a ready frame without consulting the gate.
    SendWithoutCredit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Frame {
    id: u64,
    slot: usize,
    released: bool,
    at_ms: u64,
    commit_no: u64,
}

#[derive(Clone, Copy, Debug)]
struct Infl {
    n: u64,
    at: u64,
    reads: Option<u64>,
    slot: usize,
}

#[derive(Clone, Copy, Debug)]
struct Item {
    need: u64,
    latch: Option<u64>,
}

#[derive(Clone, Debug, Default)]
struct Share {
    count: u64,
    seen: u64,
    latest_seq: u32,
    want: Option<u32>,
}

/// Everything the invariants need that the components do not keep (never part of the state key).
#[derive(Clone, Debug, Default)]
struct Obs {
    // relay, per connection
    offered: u64,
    last_acc_relay: Option<u32>,
    newest_want: Option<u32>,
    last_label: u32,
    last_answer_ms: Option<u64>,
    opened_now: bool,
    // gate
    gate_wants: u64,
    reset_dropped: u64,
    not_before: u64,
    // vblank
    last_vblank: u64,
    last_tick_stamp: u64,
    ticks: u64,
    // source
    events_since_fallback: u64,
    fallbacks: u64,
    to_broker: u64,
    to_free: u64,
    // frames
    frames: u64,
    commits: u64,
    copies: u64,
    flips: u64,
    // set when a tick was emitted in the current step
    emitted_now: bool,
    // copy credits that reached the mailbox and were not yet spent
    credits_avail: i64,
    // since when (ms) a frame has waited in the ring's ready slot
    ready_since_ms: Option<u64>,
}

#[derive(Clone)]
struct World {
    c: Cfg,
    now: u64,
    pacer: Pacer,
    gate: CreditGate,
    share: Share,
    // worker
    mailbox: Mailbox,
    started: u64,
    done: u64,
    barrier: u64,
    inflight: Option<Infl>,
    queue: VecDeque<Item>,
    latches: u64,
    pending_vsync: bool,
    // guest
    outstanding: Option<bool>,
    latched: bool,
    // ring
    front: Option<usize>,
    ready: Option<Frame>,
    held: Vec<Frame>,
    next_id: u64,
    // relay
    negotiated: bool,
    reqs: Requests,
    last_rx_ms: Option<u64>,
    credit: bool,
    credit_at: Option<u64>,
    last_commit: Option<Frame>,
    commits: u64,
    // broker
    b_next: u32,
    b_last: Option<(u32, bool)>,
    obs: Obs,
}

fn next_seq(s: u32) -> u32 {
    match s.wrapping_add(1) {
        0 => 1,
        n => n,
    }
}

impl World {
    fn new(c: Cfg) -> World {
        let mut pacer = Pacer::new(75);
        pacer.on_heads(
            &[HeadMode {
                head: 0,
                period_ns: P,
                raster: (2200, 1125),
            }],
            T0,
        );
        let mut w = World {
            c,
            now: T0,
            pacer,
            gate: CreditGate::new(),
            share: Share::default(),
            mailbox: Mailbox::default(),
            started: 0,
            done: 0,
            barrier: 0,
            inflight: None,
            queue: VecDeque::new(),
            latches: 0,
            pending_vsync: false,
            outstanding: None,
            latched: false,
            front: None,
            ready: None,
            held: Vec::new(),
            next_id: 1,
            negotiated: true,
            reqs: Requests::new(),
            last_rx_ms: None,
            credit: true,
            credit_at: None,
            last_commit: None,
            commits: 0,
            b_next: c.start_seq,
            b_last: None,
            obs: Obs::default(),
        };
        w.obs.last_tick_stamp = T0;
        w
    }

    fn ms(&self) -> u64 {
        self.now / MS
    }

    fn live(&self) -> bool {
        self.negotiated && tick_live(self.last_rx_ms, self.ms())
    }

    // ---------------------------------------------------------------------------------------------
    // the relay (conn.rs, modelled)

    fn free_slot(&self) -> Option<usize> {
        (0..SLOTS).find(|s| {
            self.front != Some(*s)
                && self.ready.is_none_or(|f| f.slot != *s)
                && self.held.iter().all(|f| f.slot != *s)
                && self.inflight.is_none_or(|i| i.slot != *s)
        })
    }

    fn relay_progress(&mut self) -> Result<(), String> {
        let ms = self.ms();
        let open = self.reqs.open();
        let gate = if self.c.bug == Some(Bug::SendWithoutCredit) {
            FrameGate::Send
        } else {
            frame_gate(self.negotiated, self.live(), open, self.credit)
        };
        match gate {
            FrameGate::Send => {}
            FrameGate::WaitRequest => {
                if self.ready.is_some() {
                    self.reqs.blocked();
                }
                return Ok(());
            }
            FrameGate::WaitCredit => return Ok(()),
        }
        if self.ready.is_none() {
            return Ok(());
        }
        // take_broker: Full while HELD_CAP are held — a released-retained frame yields, then a reclaim
        if self.held.len() as u32 >= HELD_CAP {
            if let Some(i) = self.held.iter().position(|f| f.released) {
                self.held.remove(i);
            } else if !self.reclaim(ms) {
                return Ok(());
            }
        }
        let Some(f) = self.ready.take() else {
            return Ok(());
        };
        self.obs.ready_since_ms = None;
        self.held.push(f);
        // ---- commit
        let live = self.live();
        let open = self.reqs.open();
        if live && open.is_none() {
            return Err(format!(
                "a frame was sent WITHOUT a credit: ticks live, no request open (now {} ms)",
                ms
            ));
        }
        let label = if self.negotiated { open.unwrap_or(0) } else { 0 };
        if label != 0 {
            if label == self.obs.last_label {
                return Err(format!("request {label} answered by a SECOND frame"));
            }
            if Some(label) != self.obs.newest_want {
                return Err(format!(
                    "a frame answered request {label}, not the newest issued {:?}",
                    self.obs.newest_want
                ));
            }
            self.obs.last_label = label;
        }
        if self.negotiated {
            let _ = self.reqs.spend(ms);
            if label != 0 {
                self.obs.last_answer_ms = Some(ms);
            }
        }
        self.commits += 1;
        self.obs.commits += 1;
        self.credit = false;
        self.credit_at = Some(ms + BACKSTOP_MS);
        let mut f = f;
        f.at_ms = ms;
        f.commit_no = self.commits;
        if let Some(h) = self.held.last_mut() {
            *h = f;
        }
        self.last_commit = Some(f);
        if self.held.len() > 2 {
            return Err(format!("{} frames in flight (> 2)", self.held.len()));
        }
        Ok(())
    }

    fn reclaim(&mut self, ms: u64) -> bool {
        let last = self.last_commit.map(|f| f.id);
        let commits = self.commits;
        let before = self.held.len();
        self.held.retain(|f| {
            !(Some(f.id) != last && ms >= f.at_ms + RECLAIM_MS && commits > f.commit_no)
        });
        self.held.len() < before
    }

    fn relay_on_tick(&mut self, t: TickPkt) -> Result<(), String> {
        let ms = self.ms();
        self.obs.offered += 1;
        let expect_fresh = self
            .obs
            .last_acc_relay
            .is_none_or(|l| seq_newer(t.tick_seq, l));
        match self.reqs.on_tick(&t) {
            TickVerdict::Stale => {
                if expect_fresh {
                    return Err(format!("a fresh tick {} was dropped as stale", t.tick_seq));
                }
            }
            TickVerdict::Fresh { opened, .. } => {
                if !expect_fresh {
                    return Err(format!(
                        "a STALE tick {} (last accepted {:?}) was accepted",
                        t.tick_seq, self.obs.last_acc_relay
                    ));
                }
                self.obs.last_acc_relay = Some(t.tick_seq);
                self.last_rx_ms = Some(ms);
                if t.want {
                    self.obs.newest_want = Some(t.tick_seq);
                }
                self.obs.opened_now = opened;
                // the share
                self.share.count += 1;
                self.share.latest_seq = t.tick_seq;
                if t.want {
                    self.share.want = Some(t.tick_seq);
                }
                self.relay_progress()?;
                self.maybe_no_change()?;
                self.obs.opened_now = false;
            }
        }
        Ok(())
    }

    fn maybe_no_change(&mut self) -> Result<(), String> {
        let ms = self.ms();
        let nothing = self.ready.is_none();
        let Some(r) = self.reqs.idle_request(nothing, ms) else {
            return Ok(());
        };
        if self.obs.opened_now {
            return Err("NO_CHANGE sent at the tick that opened the request".into());
        }
        if self.ready.is_some() {
            return Err("NO_CHANGE sent while a frame is ready to send".into());
        }
        let gap = (P / 2_000_000).max(1);
        if self.obs.last_answer_ms.is_some_and(|t| ms < t + gap) {
            return Err(format!(
                "NO_CHANGE {} ms after the last answer (the gap is {gap})",
                ms - self.obs.last_answer_ms.unwrap_or(0)
            ));
        }
        if Some(r) != self.obs.newest_want {
            return Err("NO_CHANGE for a request that is not the newest".into());
        }
        self.reqs.no_change_done(ms);
        self.obs.last_answer_ms = Some(ms);
        Ok(())
    }

    fn relay_finish(&mut self) -> Result<(), String> {
        let ms = self.ms();
        if !self.credit && self.credit_at.is_some_and(|t| ms >= t) {
            self.credit = true;
            self.credit_at = None;
            self.relay_progress()?;
        }
        if self.reclaim(ms) {
            self.relay_progress()?;
        }
        // the ticks went silent: the legacy credit takes over at once
        if self.negotiated && !self.live() && self.last_rx_ms.is_some() {
            self.relay_progress()?;
        }
        Ok(())
    }

    fn release_oldest(&mut self) -> Result<(), String> {
        if self.held.is_empty() {
            return Ok(());
        }
        let f = self.held[0];
        if self.last_commit.map(|l| l.id) == Some(f.id) {
            self.held[0].released = true;
            self.credit = true;
            self.credit_at = None;
        } else {
            self.held.remove(0);
        }
        self.relay_progress()
    }

    fn ev_frame(&mut self) -> Result<(), String> {
        self.credit = true;
        self.credit_at = None;
        self.relay_progress()
    }

    fn reconnect(&mut self) -> Result<(), String> {
        let last = self.last_commit;
        self.held.clear();
        if let Some(f) = last
            && self.ready.is_none()
        {
            let mut f = f;
            f.released = false;
            self.ready = Some(f);
            self.obs.ready_since_ms = Some(self.ms());
        }
        self.share.want = None;
        if self.c.reset_gate {
            self.obs.reset_dropped += u64::from(self.gate.pending().is_some());
            self.gate.reset();
        }
        self.reqs = Requests::new();
        self.last_rx_ms = None;
        self.credit = true;
        self.credit_at = None;
        self.last_commit = None;
        self.obs.offered = 0;
        self.obs.last_acc_relay = None;
        self.obs.newest_want = None;
        self.obs.last_label = 0;
        self.obs.last_answer_ms = None;
        self.b_next = self.c.start_seq;
        self.b_last = None;
        // the replay goes out BEFORE the opt-in (legacy, unlabelled), then CAPS negotiates
        self.negotiated = false;
        self.relay_progress()?;
        self.negotiated = true;
        self.relay_progress()
    }

    // ---------------------------------------------------------------------------------------------
    // the display worker (display.rs, modelled around the real pure components)

    fn latch(&mut self) {
        let idx = self.latches;
        self.latches += 1;
        self.mailbox.latch();
        self.barrier = flip_completion_need(self.barrier, self.inflight.map(|f| f.n));
        self.queue.push_back(Item {
            need: self.barrier,
            latch: Some(idx),
        });
        self.latched = true;
    }

    fn worker_pass(&mut self) -> Result<(), String> {
        for _ in 0..4 {
            let before = (
                self.started,
                self.done,
                self.queue.len(),
                self.obs.ticks,
                self.share.seen,
            );
            self.worker_once()?;
            if before
                == (
                    self.started,
                    self.done,
                    self.queue.len(),
                    self.obs.ticks,
                    self.share.seen,
                )
            {
                break;
            }
        }
        Ok(())
    }

    fn worker_once(&mut self) -> Result<(), String> {
        let now = self.now;
        // 1. the free clock's ticks, and the silence's end
        let mut ticks = self.pacer.due(now);
        // 2. the broker's latest tick and newest request, through the gate
        if self.share.count != self.share.seen {
            let coalesced = self.share.count.saturating_sub(self.share.seen + 1);
            self.share.seen = self.share.count;
            let want = self.share.want.take();
            let seq = self.share.latest_seq;
            let fresh = self.gate.fresh(seq);
            if !fresh {
                return Err(format!(
                    "the worker dropped tick {seq} as stale although the relay accepted it \
                     (a new connection restarts the numbers: the gate was not reset)"
                ));
            }
            let t = self.pacer.broker_tick(0, now);
            if t.is_some() || self.pacer.source(0).is_some() {
                self.obs.events_since_fallback += 1;
            }
            let (last, guard) = self
                .pacer
                .source(0)
                .map_or((None, 0), |s| (s.last_tick_ns(), s.guard_ns()));
            if let Some(r) = want {
                self.obs.gate_wants += 1;
                self.obs.not_before = if t.is_some() {
                    now
                } else {
                    last.map_or(0, |l| l + guard)
                };
                let _ = r;
            }
            let ok = self
                .gate
                .on_tick(seq, coalesced, want, t.map(|_| now), last, guard);
            if !ok {
                return Err("the gate called a fresh event stale".into());
            }
            ticks.extend(t);
        }
        // 3. the gate hands a usable credit to the mailbox, once, never before its edge
        let edge = self.gate.next_ns();
        if self.gate.poll(now).is_some() {
            if edge.is_none_or(|e| e > now) {
                return Err("a credit was granted before it was usable".into());
            }
            if now < self.obs.not_before {
                return Err(format!(
                    "a credit was granted {} ns before the guard's edge",
                    self.obs.not_before - now
                ));
            }
            let had = self.mailbox.credit_held();
            self.mailbox.grant();
            self.obs.credits_avail += i64::from(!had);
        }
        // 4. the ticks: vblank, the vsync flip latches, a free tick grants a credit by itself
        for t in ticks {
            let src = self.pacer.source(0).map(crate::tick::TickSource::last_tick_ns);
            let stamp = src.flatten().unwrap_or(now);
            if t.seq != self.obs.last_vblank + 1 {
                return Err(format!(
                    "the vblank number went {} -> {}",
                    self.obs.last_vblank, t.seq
                ));
            }
            if self.obs.ticks > 0 && stamp < self.obs.last_tick_stamp {
                return Err("a tick's stamp went backwards".into());
            }
            if self.obs.ticks > 0 && stamp - self.obs.last_tick_stamp < GUARD {
                return Err(format!(
                    "two ticks {} ns apart (guard {GUARD}): a double tick",
                    stamp - self.obs.last_tick_stamp
                ));
            }
            self.obs.last_vblank = t.seq;
            self.obs.last_tick_stamp = stamp;
            self.obs.ticks += 1;
            self.obs.emitted_now = true;
            if t.from == Source::Free {
                let had = self.mailbox.credit_held();
                self.mailbox.tick();
                self.obs.credits_avail += i64::from(!had);
            }
            if self.pending_vsync {
                self.pending_vsync = false;
                self.latch();
            }
        }
        for (_, ho, _) in self.pacer.take_handoffs() {
            match ho {
                Handoff::ToBroker => {
                    self.obs.to_broker += 1;
                    if self.obs.fallbacks > 0 && self.obs.events_since_fallback < 2 {
                        return Err(
                            "the tick went back to the broker on its FIRST event after a fallback"
                                .into(),
                        );
                    }
                }
                Handoff::ToFree => {
                    self.obs.to_free += 1;
                    self.obs.fallbacks += 1;
                    self.obs.events_since_fallback = 0;
                }
            }
        }
        // 5. the copy in flight completes: the frame is published to the broker's ring
        if let Some(i) = self.inflight
            && i.at <= now
        {
            self.inflight = None;
            self.done = i.n;
            let id = self.next_id;
            self.next_id += 1;
            self.front = Some(i.slot);
            if self.ready.is_none() {
                self.obs.ready_since_ms = Some(self.ms());
            }
            self.ready = Some(Frame {
                id,
                slot: i.slot,
                released: false,
                at_ms: 0,
                commit_no: 0,
            });
            self.obs.frames += 1;
            self.relay_progress()?;
        }
        // 6. completions whose barrier is done go out, in order
        while self.queue.front().is_some_and(|q| q.need <= self.done) {
            let Some(Item { latch, .. }) = self.queue.pop_front() else {
                break;
            };
            if let Some(j) = latch {
                if j >= 1 && self.inflight.is_some_and(|i| i.reads == Some(j - 1)) {
                    return Err(format!(
                        "BUFFER REUSE: flip {j} completed while a copy still reads flip {}'s buffer",
                        j - 1
                    ));
                }
                self.outstanding = None;
                self.latched = false;
            }
        }
        // 7. a copy starts when a frame waits, a credit is held and the stream is free
        if self.mailbox.due(self.inflight.is_some()) {
            if self.inflight.is_some() {
                return Err("two copies at once".into());
            }
            if !self.mailbox.credit_held() {
                return Err("a copy started without a credit".into());
            }
            let Some(slot) = self.free_slot() else {
                return Err("no free ring slot for a copy (a held slot would be overwritten)".into());
            };
            self.obs.credits_avail -= 1;
            if self.obs.credits_avail < 0 {
                return Err("a copy started on a credit that was already spent".into());
            }
            self.started += 1;
            self.obs.copies += 1;
            self.mailbox.copied();
            self.inflight = Some(Infl {
                n: self.started,
                at: now + COPY,
                reads: self.latches.checked_sub(1),
                slot,
            });
        }
        Ok(())
    }

    // ---------------------------------------------------------------------------------------------
    // time

    fn deadlines(&self) -> Option<u64> {
        let ms = self.ms();
        let mut c: Vec<u64> = Vec::new();
        c.extend(self.pacer.next_ns());
        c.extend(self.gate.next_ns());
        c.extend(self.inflight.map(|i| i.at));
        if let Some(r) = self.last_rx_ms
            && self.negotiated
            && tick_live(Some(r), ms)
        {
            c.push((r + BROKER_SILENCE_MS + 1) * MS);
        }
        c.extend(self.credit_at.filter(|_| !self.credit).map(|t| t * MS));
        for f in &self.held {
            if Some(f.id) != self.last_commit.map(|l| l.id) {
                c.push((f.at_ms + RECLAIM_MS) * MS);
            }
        }
        c.into_iter().filter(|t| *t > self.now).min()
    }

    fn internal(&mut self) -> Result<(), String> {
        self.relay_finish()?;
        self.worker_pass()?;
        self.relay_progress()?;
        if self.now - self.obs.last_tick_stamp > STALL_BOUND {
            return Err(format!(
                "the vblank STALLED for {} ms (bound {} ms) at {} ms",
                (self.now - self.obs.last_tick_stamp) / MS,
                STALL_BOUND / MS,
                self.ms()
            ));
        }
        Ok(())
    }

    fn advance_to(&mut self, target: u64) -> Result<(), String> {
        let mut guard = 0u32;
        while let Some(t) = self.deadlines() {
            if t > target {
                break;
            }
            guard += 1;
            if guard > 200_000 {
                return Err("time does not advance".into());
            }
            self.now = t;
            self.internal()?;
        }
        if target > self.now {
            self.now = target;
        }
        self.internal()
    }

    // ---------------------------------------------------------------------------------------------
    // the environment

    fn pkt(&self, seq: u32, want: bool) -> TickPkt {
        TickPkt {
            tick_seq: seq,
            refresh_ns: P as u32,
            want,
            clock_only: false,
            present_ns: self.now,
        }
    }

    fn broker_tick(&mut self, skip: u32, want: bool) -> Result<(), String> {
        let mut s = self.b_next;
        for _ in 0..skip {
            s = next_seq(s);
        }
        self.b_next = next_seq(s);
        self.b_last = Some((s, want));
        let p = self.pkt(s, want);
        self.relay_on_tick(p)
    }

    fn apply(&mut self, l: L) -> Result<(), String> {
        self.obs.emitted_now = false;
        let r = self.apply_inner(l);
        r?;
        self.check()
    }

    fn apply_inner(&mut self, l: L) -> Result<(), String> {
        match l {
            L::Adv(d) => self.advance_to(self.now + d)?,
            L::AdvNext => {
                if let Some(t) = self.deadlines() {
                    self.advance_to(t)?;
                }
            }
            L::AdvCopy(off) => {
                let t = self.inflight.map_or(self.now + MS, |i| i.at);
                let t = (t as i64 + off).max(self.now as i64 + 1) as u64;
                self.advance_to(t)?;
            }
            L::AdvGuard(off) => {
                let t = self.obs.last_tick_stamp as i64 + GUARD as i64 + off;
                self.advance_to(t.max(self.now as i64 + 1) as u64)?;
            }
            L::AdvSilence(off) => {
                let base = match self.pacer.source(0) {
                    Some(s) if s.source() == Source::Broker => s.deadline_ns(),
                    _ => self
                        .last_rx_ms
                        .map_or(self.now + MS, |r| (r + BROKER_SILENCE_MS) * MS),
                };
                let t = (base as i64 + off).max(self.now as i64 + 1) as u64;
                self.advance_to(t)?;
            }
            L::FlipImm => {
                if self.outstanding.is_some() {
                    return Ok(());
                }
                self.obs.flips += 1;
                self.outstanding = Some(true);
                self.latch();
                self.worker_pass()?;
            }
            L::FlipVsync => {
                if self.outstanding.is_some() {
                    return Ok(());
                }
                self.obs.flips += 1;
                self.outstanding = Some(false);
                self.latched = false;
                self.pending_vsync = true;
                self.worker_pass()?;
            }
            L::Tick { skip, want } => {
                self.broker_tick(skip, want)?;
                self.worker_pass()?;
            }
            L::Dup => {
                if let Some((s, w)) = self.b_last {
                    let p = self.pkt(s, w);
                    self.relay_on_tick(p)?;
                    self.worker_pass()?;
                }
            }
            L::Stale => {
                // two numbers behind the newest sent
                if let Some((s, w)) = self.b_last {
                    let old = s.wrapping_sub(2).max(1);
                    let p = self.pkt(old, w);
                    self.relay_on_tick(p)?;
                    self.worker_pass()?;
                }
            }
            L::Burst { n, want } => {
                for i in 0..n {
                    self.broker_tick(0, want || i == 0 && want)?;
                }
                self.worker_pass()?;
            }
            L::Rate { mult, ms, want } => {
                let step = P / u64::from(mult);
                let end = self.now + ms * MS;
                while self.now + step <= end {
                    self.advance_to(self.now + step)?;
                    self.broker_tick(0, want)?;
                    self.worker_pass()?;
                }
            }
            L::Release => {
                self.release_oldest()?;
                self.worker_pass()?;
            }
            L::EvFrame => {
                self.ev_frame()?;
                self.worker_pass()?;
            }
            L::Reconnect => {
                self.reconnect()?;
                self.worker_pass()?;
            }
            L::Flap(kind) => {
                let gaps: &[u64] = match kind {
                    0 => &[55, 55, 55, 55, 55, 55],
                    _ => &[40, 60, 40, 60, 40, 60],
                };
                for g in gaps {
                    self.broker_tick(0, false)?;
                    self.worker_pass()?;
                    self.advance_to(self.now + g * MS)?;
                }
            }
        }
        Ok(())
    }

    // ---------------------------------------------------------------------------------------------
    // the invariants that hold in EVERY state

    fn check(&self) -> Result<(), String> {
        // 1. no flip parked
        match self.outstanding {
            Some(true) if !self.latched => return Err("an immediate flip is not latched".into()),
            Some(false) if !self.latched && !self.pending_vsync => {
                return Err("a vsync flip is neither latched nor waiting for its tick".into());
            }
            _ => {}
        }
        if self.pending_vsync && self.obs.emitted_now {
            return Err("a vsync flip stayed parked past a tick".into());
        }
        // 2. the queue
        if let Some(q) = self.queue.front()
            && q.need <= self.done
        {
            return Err("a completion waits behind a satisfied barrier".into());
        }
        if self.queue.iter().any(|q| q.need > self.started) {
            return Err("a completion waits for a copy that never started".into());
        }
        // 5. frames in flight, the ring
        if self.held.len() > 2 {
            return Err(format!("{} frames in flight (> 2)", self.held.len()));
        }
        if let (Some(r), true) = (self.ready, !self.held.is_empty())
            && self.held.iter().any(|f| f.slot == r.slot)
        {
            return Err("the ready frame shares a slot with a held one".into());
        }
        // a ready frame is never stuck behind a SILENT broker: ticks silent (> 50 ms), room to send
        // (fewer than the cap in flight, or a retained one that yields), and the legacy backstop (100 ms)
        // has had time — it goes out
        if let Some(since) = self.obs.ready_since_ms
            && self.ready.is_some()
            && (self.held.len() as u32) < HELD_CAP
            && !(self.negotiated && tick_live(self.last_rx_ms, self.ms()))
            && self.ms() > since + BACKSTOP_MS + BROKER_SILENCE_MS + 2
            && self.last_rx_ms.is_none_or(|r| self.ms() > r + BACKSTOP_MS + BROKER_SILENCE_MS + 2)
        {
            return Err(format!(
                "a frame is STUCK in the ring for {} ms behind a silent broker",
                self.ms() - since
            ));
        }
        // 10. counters
        let c = self.reqs.c;
        if c.ticks + c.ticks_stale != self.obs.offered {
            return Err(format!(
                "ticks offered {} != accepted {} + stale {}",
                self.obs.offered, c.ticks, c.ticks_stale
            ));
        }
        if c.want_ticks != c.frames_on_credit + c.no_change_sent + c.credits_coalesced + u64::from(self.reqs.open().is_some())
        {
            return Err(format!(
                "wants {} != frames {} + no_change {} + replaced {} + open {:?}",
                c.want_ticks,
                c.frames_on_credit,
                c.no_change_sent,
                c.credits_coalesced,
                self.reqs.open()
            ));
        }
        let g = self.gate.c;
        let pending = u64::from(self.gate.pending().is_some());
        if self.obs.gate_wants
            != g.granted + g.credits_coalesced + g.stale_credits + pending + self.obs.reset_dropped
        {
            return Err(format!(
                "gate wants {} != granted {} + replaced {} + stale {} + pending {pending} + reset {}",
                self.obs.gate_wants, g.granted, g.credits_coalesced, g.stale_credits, self.obs.reset_dropped
            ));
        }
        // 4. a copy needs a credit: copies <= credits that reached the mailbox
        if self.mailbox.copies != self.obs.copies {
            return Err("copies counted twice".into());
        }
        Ok(())
    }

    /// The behaviour-relevant state, times relative to `now`, as a hash.
    fn key(&self) -> u64 {
        let now = self.now as i128;
        let rel = |t: u64| (t as i128 - now).clamp(-3_000 * MS as i128, 20_000 * MS as i128) as i64;
        let relo = |t: Option<u64>| t.map_or(i64::MIN, rel);
        let base = i64::from(self.b_next);
        let sq = |s: Option<u32>| s.map_or(i64::MIN, |s| base - i64::from(s));
        let mut k: Vec<i64> = Vec::with_capacity(64);
        let (br, at, lt, lb, st, fr, pg, pl, bs) = self.pacer.source(0).map_or(
            (false, 0, None, None, 0, 0, 0, None, None),
            crate::tick::TickSource::probe_state,
        );
        k.extend([
            i64::from(br),
            rel(at),
            relo(lt),
            relo(lb),
            rel(st),
            i64::from(fr),
            i64::from(pg),
            relo(pl),
            relo(bs),
        ]);
        let (gl, gr, gp) = self.gate.probe_state();
        k.extend([sq(gl), sq(gr), gp.map_or(i64::MIN, |(r, a)| base - i64::from(r) + rel(a) * 1_000_003)]);
        let (rl, ro, rf, ra, rs) = self.reqs.probe_state();
        k.extend([
            sq(rl),
            sq(ro),
            i64::from(rf),
            ra.map_or(i64::MIN, |t| (self.ms() as i64 - t as i64).min(200)),
            i64::from(rs.min(2)),
        ]);
        k.extend([
            self.last_rx_ms
                .map_or(i64::MIN, |t| (self.ms() as i64 - t as i64).min(200)),
            i64::from(self.credit),
            self.credit_at
                .map_or(i64::MIN, |t| t as i64 - self.ms() as i64),
            (self.now % MS) as i64,
            i64::from(self.negotiated),
            i64::from(self.mailbox.pending()),
            i64::from(self.mailbox.credit_held()),
            (self.share.count - self.share.seen).min(3) as i64,
            sq(self.share.want),
            sq(Some(self.share.latest_seq)),
            self.inflight.map_or(i64::MIN, |i| {
                rel(i.at) + 7 * i64::try_from(self.latches - i.reads.map_or(0, |r| r + 1)).unwrap_or(0)
            }),
            self.inflight.map_or(i64::MIN, |i| i.slot as i64),
            i64::from(self.pending_vsync),
            self.outstanding.map_or(-1, i64::from),
            i64::from(self.latched),
            self.started as i64 - self.done as i64,
            self.barrier as i64 - self.done as i64,
        ]);
        for q in &self.queue {
            k.push(q.need as i64 - self.done as i64);
            k.push(q.latch.map_or(-1, |l| (self.latches - l) as i64));
        }
        k.push(-7);
        k.push(self.front.map_or(-1, |s| s as i64));
        k.push(self.ready.map_or(-1, |f| f.slot as i64));
        for f in &self.held {
            k.push(f.slot as i64);
            k.push(i64::from(f.released));
            k.push((self.ms() as i64 - f.at_ms as i64).min(1100));
            k.push((self.commits - f.commit_no).min(3) as i64);
        }
        k.push(-8);
        k.push(
            self.last_commit
                .map_or(-1, |f| self.held.iter().position(|h| h.id == f.id).map_or(-2, |p| p as i64)),
        );
        k.push(sq(self.b_last.map(|(s, _)| s)));
        k.push(i64::from(self.b_last.is_some_and(|(_, w)| w)));
        k.push(sq(self.obs.newest_want));
        k.push(i64::from(self.obs.last_label != 0 && Some(self.obs.last_label) == self.reqs.open()));
        k.push(sq(Some(self.obs.last_label).filter(|l| *l != 0)));
        k.push(sq(self.obs.last_acc_relay));
        k.push(self.obs.last_answer_ms.map_or(i64::MIN, |t| (self.ms() as i64 - t as i64).min(200)));
        k.push(i64::from(self.obs.fallbacks.min(2) as u8));
        k.push(self.obs.events_since_fallback.min(3) as i64);
        k.push(i64::from(self.obs.to_broker.min(2) as u8));
        k.push(rel(self.obs.last_tick_stamp));
        k.push(rel(self.obs.not_before));
        let mut h = std::collections::hash_map::DefaultHasher::new();
        k.hash(&mut h);
        h.finish()
    }
}

// -------------------------------------------------------------------------------------------------
// the alphabet

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum L {
    /// Advance by this many ns (internal deadlines fire on the way).
    Adv(u64),
    /// To the next internal deadline.
    AdvNext,
    /// To the copy's end, plus `off` ns.
    AdvCopy(i64),
    /// To the guard's edge after the last tick, plus `off`.
    AdvGuard(i64),
    /// To the broker's silence edge, plus `off`.
    AdvSilence(i64),
    FlipImm,
    FlipVsync,
    /// A fresh tick (skipping `skip` numbers).
    Tick { skip: u32, want: bool },
    Dup,
    Stale,
    /// `n` fresh ticks in one instant.
    Burst { n: u32, want: bool },
    /// Ticks at `mult` times the refresh for `ms` milliseconds.
    Rate { mult: u32, ms: u64, want: bool },
    Release,
    EvFrame,
    Reconnect,
    Flap(u8),
}

const TW: L = L::Tick { skip: 0, want: true };
const TN: L = L::Tick { skip: 0, want: false };

#[derive(Debug)]
struct Explored {
    states: usize,
    transitions: usize,
}

#[derive(Debug)]
struct Counterexample {
    path: Vec<L>,
    why: String,
}

/// Depth-first over every ordering of `letters` up to `depth`, pruned by the visited set (a state is
/// re-expanded only if reached with more depth left).
fn explore(root: &World, letters: &[L], depth: usize) -> Result<Explored, Counterexample> {
    let mut seen: HashMap<u64, usize> = HashMap::new();
    let mut transitions = 0usize;
    let mut stack: Vec<(World, usize, Vec<L>)> = vec![(root.clone(), depth, Vec::new())];
    seen.insert(root.key(), depth);
    while let Some((w, left, path)) = stack.pop() {
        if left == 0 {
            continue;
        }
        for &l in letters {
            let mut n = w.clone();
            transitions += 1;
            if let Err(why) = n.apply(l) {
                let mut path = path.clone();
                path.push(l);
                return Err(Counterexample { path, why });
            }
            let k = n.key();
            if seen.get(&k).is_some_and(|d| *d >= left - 1) {
                continue;
            }
            seen.insert(k, left - 1);
            let mut p = path.clone();
            p.push(l);
            stack.push((n, left - 1, p));
        }
    }
    Ok(Explored {
        states: seen.len(),
        transitions,
    })
}

fn root(reset_gate: bool, start_seq: u32) -> World {
    World::new(Cfg {
        start_seq,
        reset_gate,
        bug: None,
    })
}

fn run(name: &str, w: &World, letters: &[L], depth: usize) -> Explored {
    let t = std::time::Instant::now();
    match explore(w, letters, depth) {
        Ok(e) => {
            eprintln!(
                "lockstep_exhaustive {name}: depth {depth}, {} letters, {} states, {} transitions, {:?}",
                letters.len(),
                e.states,
                e.transitions,
                t.elapsed()
            );
            e
        }
        Err(c) => panic!("{name}: {} after {:?}", c.why, c.path),
    }
}

// -------------------------------------------------------------------------------------------------
// the explorations

/// ★ FRAMES: flips, ticks, the copy, the broker's releases — at least two complete frames (a credit, a
/// flip, a copy, a send, a release each) with the copy ending exactly at, before and after the next tick.
#[test]
fn lockstep_exhaustive_frames() {
    let letters = [
        L::FlipImm,
        L::FlipVsync,
        TW,
        TN,
        L::Release,
        L::EvFrame,
        L::AdvCopy(0),
        L::AdvCopy(-1),
        L::AdvGuard(0),
        L::Adv(P),
        L::AdvNext,
    ];
    run("frames", &root(true, 1), &letters, 8);
}

/// ★ TIME AND SILENCE: every boundary of the 50 ms fallback and the handoff back, stalls of 10 ms to
/// 10 s, flapping, a tick the same instant as the takeover, a tick mid-copy.
#[test]
fn lockstep_exhaustive_time_and_silence() {
    let letters = [
        TW,
        TN,
        L::FlipImm,
        L::FlipVsync,
        L::AdvSilence(-1),
        L::AdvSilence(0),
        L::AdvSilence(1),
        L::AdvGuard(-1),
        L::AdvGuard(0),
        L::AdvCopy(0),
        L::Adv(10 * MS),
        L::Adv(200 * MS),
        L::Flap(0),
        L::Flap(1),
    ];
    run("time_and_silence", &root(true, 1), &letters, 6);
}

/// ★ LONG STALLS: the broker silent for 2 s and 10 s (hundreds of free ticks on the way), then back.
#[test]
fn lockstep_exhaustive_long_stalls() {
    let letters = [
        TW,
        TN,
        L::FlipImm,
        L::FlipVsync,
        L::Dup,
        L::Stale,
        L::AdvSilence(0),
        L::Adv(2_000 * MS),
        L::Adv(10_000 * MS),
        L::Release,
    ];
    run("long_stalls", &root(true, 1), &letters, 5);
}

/// ★ HOSTILE ORDERS AND FLOODS: skipping, duplicate and stale numbers, bursts of 10/100/1000 ticks in
/// one instant, sustained floods at 10x/100x/1000x, reconnects, with the broker never releasing.
#[test]
fn lockstep_exhaustive_hostile_and_floods() {
    let letters = [
        TW,
        TN,
        L::Tick { skip: 3, want: true },
        L::Dup,
        L::Stale,
        L::Burst { n: 10, want: true },
        L::Burst { n: 100, want: true },
        L::Burst { n: 1000, want: false },
        L::Rate { mult: 10, ms: 40, want: true },
        L::Rate { mult: 100, ms: 20, want: false },
        L::Rate { mult: 1000, ms: 8, want: true },
        L::FlipImm,
        L::Reconnect,
        L::Release,
        L::AdvCopy(0),
        L::Adv(P),
        L::AdvSilence(0),
    ];
    run("hostile_and_floods", &root(true, 1), &letters, 5);
}

/// ★ THE WRAP: the broker's numbers start two before `u32::MAX` and cross 0 (never sent) mid-run.
#[test]
fn lockstep_exhaustive_the_sequence_wrap() {
    let letters = [
        TW,
        TN,
        L::Tick { skip: 1, want: true },
        L::Dup,
        L::Stale,
        L::FlipImm,
        L::AdvCopy(0),
        L::Adv(P),
        L::Reconnect,
        L::Release,
    ];
    run("sequence_wrap", &root(true, u32::MAX - 1), &letters, 6);
}

/// ★ LONG FRAMES: ten letters of frames only — the pipeline of several frames with a broker that
/// never releases, releases late, or at once (3 frames could be in flight if the bound failed).
#[test]
fn lockstep_exhaustive_three_frames_and_the_in_flight_bound() {
    let letters = [L::FlipImm, TW, L::AdvCopy(0), L::Release, L::Adv(2 * MS)];
    run("three_frames", &root(true, 1), &letters, 10);
}

/// The bound is the named constant, and the invariants hold it as the literal 2.
#[test]
fn lockstep_exhaustive_held_cap_is_two() {
    assert_eq!(HELD_CAP, 2);
    assert_eq!(kf_broker::ticks::MAX_FRAMES_IN_FLIGHT, 2);
}

/// The DEEP runs (ignored by default; `cargo test --release -p kf-disp --lib lockstep_exhaustive_deep -- --ignored`):
/// the same alphabets one or two letters deeper (minutes).
#[test]
#[ignore = "deep: minutes"]
fn lockstep_exhaustive_deep_frames() {
    let l = [L::FlipImm, L::FlipVsync, TW, TN, L::Release, L::EvFrame, L::AdvCopy(0), L::AdvCopy(-1), L::AdvGuard(0), L::Adv(P), L::AdvNext];
    run("deep frames", &root(true, 1), &l, 10);
}

#[test]
#[ignore = "deep: minutes"]
fn lockstep_exhaustive_deep_time_and_silence() {
    let l = [TW, TN, L::FlipImm, L::FlipVsync, L::AdvSilence(-1), L::AdvSilence(0), L::AdvSilence(1), L::AdvGuard(-1), L::AdvGuard(0), L::AdvCopy(0), L::Adv(10 * MS), L::Adv(200 * MS), L::Adv(2_000 * MS), L::Adv(10_000 * MS), L::Flap(0), L::Flap(1)];
    run("deep time_and_silence", &root(true, 1), &l, 8);
}

// -------------------------------------------------------------------------------------------------
// the gaps the explorer FOUND (kept as tests, so they stay found)

/// ★ FINDING 1. After a reconnect the broker's numbers restart at 1; the display worker's gate still
/// holds the old connection's last number, so it drops every new tick as stale — the broker never ticks
/// the guest again until the numbers pass the old ones, and its credits are dropped too. The model with
/// the gate reset on a connection change (the design: `reset_gate`) is clean (all tests above); without
/// it, the explorer finds this within three letters.
#[test]
fn lockstep_exhaustive_a_reconnect_without_a_gate_reset_drops_the_new_connections_ticks() {
    let w = root(false, 1);
    let c = explore(&w, &[TW, L::Burst { n: 5, want: false }, L::Reconnect, L::Adv(MS)], 4)
        .expect_err("the gap must be found");
    assert!(c.why.contains("dropped tick"), "{c:?}");
    assert!(c.path.contains(&L::Reconnect), "{c:?}");
    eprintln!("lockstep_exhaustive FINDING 1: {c:?}");
}

/// ★ FINDING 2. The strict reading of the owner's bound ("the vblank never stalls more than 50 ms + one
/// period") has a counterexample: a broker event swallowed by the guard (it comes 7.99 ms after the last
/// tick) still proves the broker alive (§5), so the silence runs from the EVENT, not from the last TICK:
/// tick, event inside the guard, silence — the guest sees up to 50 ms + one period + the guard (8 ms)
/// without a vblank. The bound the model holds is 50 ms + P + guard.
#[test]
fn lockstep_exhaustive_the_strict_stall_bound_has_a_counterexample() {
    let mut w = root(true, 1);
    w.apply(TW).unwrap();
    w.apply(L::Adv(GUARD - MS)).unwrap();
    w.apply(TN).unwrap(); // swallowed: 7 ms after the tick
    // silence runs 50 ms from that event; the clock's first tick is on the broker's grid after it
    w.apply(L::AdvSilence(0)).unwrap();
    w.apply(L::AdvNext).unwrap();
    let stall_ms = (w.now - w.obs.last_tick_stamp) / MS;
    w.apply(L::AdvNext).unwrap();
    let gap = w.obs.last_tick_stamp;
    assert!(gap > 0);
    // measure the longest gap over the whole episode
    let mut w = root(true, 1);
    w.apply(TW).unwrap();
    let t_tick = w.obs.last_tick_stamp;
    w.apply(L::Adv(GUARD - MS)).unwrap();
    w.apply(TN).unwrap();
    for _ in 0..40 {
        w.apply(L::AdvNext).unwrap();
        if w.obs.last_tick_stamp > t_tick {
            break;
        }
    }
    let longest = w.obs.last_tick_stamp - t_tick;
    eprintln!(
        "lockstep_exhaustive FINDING 2: tick, swallowed event at +{} ms, silence: next vblank {} ms later (strict bound {} ms; first look {stall_ms} ms)",
        (GUARD - MS) / MS,
        longest / MS,
        (BROKER_SILENCE_NS + P) / MS
    );
    assert!(longest > BROKER_SILENCE_NS + P, "{longest}");
    assert!(longest <= STALL_BOUND, "{longest}");
}

// -------------------------------------------------------------------------------------------------
// the explorer is not vacuous

#[test]
fn lockstep_exhaustive_the_explorer_finds_a_planted_bug() {
    let w = World::new(Cfg {
        start_seq: 1,
        reset_gate: true,
        bug: Some(Bug::SendWithoutCredit),
    });
    let c = explore(&w, &[L::FlipImm, TN, L::AdvCopy(0), TW], 6).expect_err("planted bug");
    assert!(c.why.contains("WITHOUT a credit"), "{c:?}");
    // and the same alphabet on the clean world is clean
    explore(&root(true, 1), &[L::FlipImm, TN, L::AdvCopy(0), TW], 6).expect("clean world");
}

// -------------------------------------------------------------------------------------------------
// scripted floods and stalls with EXACT counters (the same world, the same invariants)

fn tick_rate(w: &mut World, mult: u32, ms: u64, want: bool) {
    w.apply(L::Rate { mult, ms, want }).unwrap();
}

/// ★ A flood of ticks at 10x, 100x and 1000x the refresh, steady, with and without `WANT_FRAME`: the
/// vblank never ticks faster than the guard allows, at most one credit pends, and the drops are counted
/// exactly (offered = accepted + stale; every WANT is a frame, a NO_CHANGE, a replaced request or open).
#[test]
fn lockstep_exhaustive_floods_are_bounded_and_counted() {
    for mult in [10u32, 100, 1000] {
        for want in [false, true] {
            let mut w = root(true, 1);
            w.apply(L::FlipImm).unwrap();
            let ms = if mult == 1000 { 16 } else { 160 };
            tick_rate(&mut w, mult, ms, want);
            let ticks = w.obs.ticks;
            // 160 ms of a 16 ms display: 10 refreshes at most (+ the guard's slack); a 1000x flood
            // for 16 ms offers 1000 ticks and the vblank still ticked at most 2x per period
            let allowed = ms * MS / GUARD + 2;
            assert!(ticks <= allowed, "x{mult} want={want}: {ticks} vblanks > {allowed}");
            let c = w.reqs.c;
            assert_eq!(c.ticks + c.ticks_stale, w.obs.offered);
            // at most one frame per request, and one copy per credit
            assert!(w.obs.copies <= w.mailbox.copies + 1, "copies");
            assert!(w.obs.commits <= c.want_ticks + 1, "commits {} > wants {}", w.obs.commits, c.want_ticks);
            eprintln!(
                "lockstep_exhaustive flood x{mult} want={want}: offered {} vblanks {ticks} wants {} replaced {} frames {} copies {} \
                 gate(granted {} replaced {} coalesced_ticks {})",
                w.obs.offered, c.want_ticks, c.credits_coalesced, c.frames_on_credit, w.obs.copies,
                w.gate.c.granted, w.gate.c.credits_coalesced, w.gate.c.ticks_coalesced
            );
        }
    }
    // a burst of 1000 ticks in ONE instant: one vblank, one credit, one frame
    let mut w = root(true, 1);
    w.apply(L::FlipImm).unwrap();
    w.apply(L::Burst { n: 1000, want: true }).unwrap();
    assert_eq!(w.obs.ticks, 1, "one vblank for a burst");
    assert_eq!(w.reqs.c.want_ticks, 1000);
    assert_eq!(w.reqs.c.credits_coalesced, 999, "999 requests replaced, one open (or spent)");
    assert_eq!(w.gate.c.ticks_coalesced, 999, "the worker took the latest of 1000");
    assert_eq!(w.gate.c.granted, 1);
    assert_eq!(w.obs.copies, 1);
}

/// ★ STALLS: the broker stops sending (silence), then resumes with a burst of stale ticks, credits and
/// releases, after 10 ms, 49, 50, 51 ms, 200 ms, 2 s and 10 s: the guest's vblank never stops, the fallback
/// is taken exactly when due, the handoff back happens ONCE, the stale numbers are ignored.
#[test]
fn lockstep_exhaustive_stalls_resume_cleanly() {
    for (stall_ms, falls) in [(10u64, false), (49, false), (50, true), (51, true), (200, true), (2_000, true), (10_000, true)] {
        let mut w = root(true, 1);
        w.apply(TW).unwrap();
        w.apply(L::Adv(P)).unwrap();
        w.apply(TW).unwrap(); // the broker speaks, takes the tick
        assert_eq!(w.pacer.source(0).unwrap().source(), Source::Broker);
        let last_event = w.now;
        w.apply(L::Adv(stall_ms * MS)).unwrap();
        let fell = w.pacer.source(0).unwrap().source() == Source::Free;
        assert_eq!(fell, falls, "stall {stall_ms} ms (edge: the fallback is AT 50 ms)");
        let ticks_during = w.obs.ticks;
        // it resumes: two stale ticks and a duplicate, then fresh ones, then releases
        w.apply(L::Stale).unwrap();
        w.apply(L::Dup).unwrap();
        for _ in 0..4 {
            w.apply(TW).unwrap();
            w.apply(L::Adv(P)).unwrap();
        }
        w.apply(L::Release).unwrap();
        w.apply(L::Release).unwrap();
        let (to_b, to_f) = w.pacer.source(0).unwrap().handoffs();
        let expect = if falls { (2, 1) } else { (1, 0) };
        assert_eq!((to_b, to_f), expect, "stall {stall_ms} ms: handoffs (to broker, back to free)");
        assert_eq!(w.pacer.source(0).unwrap().source(), Source::Broker);
        assert!(w.reqs.c.ticks_stale >= 2, "the stale numbers were dropped and counted");
        assert!(w.now > last_event + stall_ms * MS);
        eprintln!(
            "lockstep_exhaustive stall {stall_ms} ms: fell back {fell}, vblanks during/after {ticks_during}/{}, handoffs {to_b}/{to_f}, stale {}",
            w.obs.ticks, w.reqs.c.ticks_stale
        );
    }
}

/// ★ FLAPPING around the 50 ms boundary: the tick source changes a bounded number of times — never once
/// per tick — and every change is a logged handoff (counted).
#[test]
fn lockstep_exhaustive_flapping_does_not_flap_the_source() {
    for kind in [0u8, 1] {
        let mut w = root(true, 1);
        w.apply(TW).unwrap();
        for _ in 0..30 {
            w.apply(L::Flap(kind)).unwrap();
        }
        let (a, b) = w.pacer.source(0).unwrap().handoffs();
        eprintln!("lockstep_exhaustive flap kind {kind}: 180 ticks offered, source changes {a}+{b}");
        assert!(a + b <= 8, "kind {kind}: {a}+{b} source changes for 180 ticks");
        assert_eq!(w.obs.to_broker + w.obs.to_free, a + b);
    }
}

/// ★ A broker that never reads or releases (the ring's held slots stay held): the relay blocks (counted),
/// the copy still finds a free slot, no held slot is overwritten, the guest's flips still complete, the
/// vblank runs — and the reclaim (1 s) frees what a newer commit made safe.
#[test]
fn lockstep_exhaustive_a_broker_that_never_releases_deadlocks_nothing() {
    let mut w = root(true, 1);
    for _ in 0..12 {
        w.apply(TW).unwrap();
        w.apply(L::FlipImm).unwrap();
        w.apply(L::AdvCopy(0)).unwrap();
        w.apply(L::Adv(P)).unwrap();
    }
    assert!(w.held.len() <= 2);
    assert!(w.obs.flips >= 6, "flips complete without a single RELEASE: {}", w.obs.flips);
    w.apply(L::Adv(2_000 * MS)).unwrap();
    assert!(w.held.len() <= 2);
}

/// The ALPHABET's own consistency: every letter is applicable on a fresh world without breaking an
/// invariant (a smoke test of the explorer's inputs).
#[test]
fn lockstep_exhaustive_every_letter_runs() {
    for l in [
        L::Adv(MS),
        L::AdvNext,
        L::AdvCopy(0),
        L::AdvGuard(1),
        L::AdvSilence(-1),
        L::FlipImm,
        L::FlipVsync,
        TW,
        TN,
        L::Dup,
        L::Stale,
        L::Burst { n: 3, want: true },
        L::Rate { mult: 10, ms: 20, want: true },
        L::Release,
        L::EvFrame,
        L::Reconnect,
        L::Flap(0),
        L::Flap(1),
    ] {
        let mut w = root(true, 1);
        w.apply(TW).unwrap();
        w.apply(l).unwrap_or_else(|e| panic!("{l:?}: {e}"));
    }
}

/// ★ FINDING 3. A broker flooding ticks (>= 10x the refresh) with `WANT_FRAME` while the guest flips
/// makes the COPY rate exceed the vblank rate: a tick the double-tick guard swallows still carries a
/// credit that the gate defers to the guard's edge (G2), and the next accepted tick brings its own — two
/// credits per guard interval, so about two copies per accepted tick (66-79 copies against 40 vblanks in
/// 320 ms, every 3rd millisecond-step flipping). Frames to the broker stay bounded (2 in flight, one per
/// request), the vblank stays at the guard's cap (2x the mode), and nothing blocks; but "too fast never
/// makes kayfabe go faster" is not true of the COPY. The fix is the gate's: a swallowed tick's credit
/// should wait for the next ACCEPTED tick and coalesce into its credit (copies <= vblanks). This test
/// pins today's behaviour; when the gate is fixed it fails and its assertion flips.
#[test]
fn lockstep_exhaustive_finding_3_a_flood_makes_copies_exceed_vblanks() {
    let mut out = Vec::new();
    for mult in [1u32, 2, 10, 100, 1000] {
        let mut w = root(true, 1);
        let step = P / u64::from(mult);
        let end = w.now + 320 * MS;
        let mut n = 0;
        while w.now + step <= end {
            w.apply(L::Adv(step)).unwrap();
            w.apply(L::Tick { skip: 0, want: true }).unwrap();
            if n % 3 == 0 {
                w.apply(L::FlipImm).unwrap();
            }
            n += 1;
        }
        out.push((mult, w.obs.ticks, w.obs.copies, w.reqs.c.frames_on_credit));
        eprintln!(
            "lockstep_exhaustive FINDING 3 x{mult}: vblanks {} copies {} frames {} wants {} gate granted {}",
            w.obs.ticks, w.obs.copies, w.reqs.c.frames_on_credit, w.reqs.c.want_ticks, w.gate.c.granted
        );
        assert!(w.reqs.c.frames_on_credit <= 3, "frames stay bounded: one per request, 2 in flight");
    }
    // at the display's own rate (and at 2x) copies follow the flips; under a flood they run past the vblanks
    for (mult, vbl, copies, _) in &out {
        if *mult <= 2 {
            assert!(copies <= vbl, "x{mult}: {copies} copies > {vbl} vblanks");
        } else {
            assert!(*copies > *vbl, "x{mult}: copies {copies} no longer exceed vblanks {vbl}: the gate was fixed, flip this test");
        }
    }
}

// -------------------------------------------------------------------------------------------------
// the COMPONENTS against independent oracles, exhaustively over short event sequences (they pin what the
// composed world cannot reach: a stale event at the gate, every request/NO_CHANGE rule)

#[derive(Clone, Copy, Debug)]
enum SeqPick {
    Next,
    Dup,
    Old,
    Skip5,
}

#[derive(Clone, Copy, Debug)]
enum ReqPick {
    None,
    New,
    Same,
    Older,
}

#[derive(Clone, Copy, Debug)]
enum GateOp {
    Ev { seq: SeqPick, req: ReqPick, emitted: bool },
    Poll(u64),
}

#[derive(Clone, Default, Debug, PartialEq, Eq)]
struct GateOracle {
    last_seq: Option<u32>,
    last_req: Option<u32>,
    pending: Option<(u32, u64)>,
    stale: u64,
    stale_credits: u64,
    coalesced: u64,
    deferred: u64,
    granted: u64,
    ticks_coalesced: u64,
}

const GATE_LAST_TICK: u64 = 1_000;
const GATE_GUARD: u64 = 8;
const GATE_STAMP: u64 = 2_000;

fn pick_seq(hi: Option<u32>, p: SeqPick) -> u32 {
    let h = hi.unwrap_or(0);
    match p {
        SeqPick::Next => next_seq(h),
        SeqPick::Dup => hi.unwrap_or(1),
        SeqPick::Old => h.wrapping_sub(2).max(1),
        SeqPick::Skip5 => (0..5).fold(h, |s, _| next_seq(s)),
    }
}

/// ★ The gate against its specification (G1-G3), every sequence of up to 4 operations.
#[test]
fn lockstep_exhaustive_the_gate_against_an_oracle() {
    use crate::tick::seq_newer as newer;
    let mut ops: Vec<GateOp> = Vec::new();
    for seq in [SeqPick::Next, SeqPick::Dup, SeqPick::Old, SeqPick::Skip5] {
        for req in [ReqPick::None, ReqPick::New, ReqPick::Same, ReqPick::Older] {
            for emitted in [true, false] {
                ops.push(GateOp::Ev { seq, req, emitted });
            }
        }
    }
    for t in [GATE_LAST_TICK + GATE_GUARD - 1, GATE_LAST_TICK + GATE_GUARD, GATE_STAMP - 1, GATE_STAMP] {
        ops.push(GateOp::Poll(t));
    }
    fn go(
        ops: &[GateOp],
        depth: usize,
        g: &CreditGate,
        o: &GateOracle,
        hi: Option<u32>,
        n: &mut u64,
    ) {
        if depth == 0 {
            return;
        }
        for &op in ops {
            let (mut g, mut o, mut hi2) = (g.clone(), o.clone(), hi);
            match op {
                GateOp::Ev { seq, req, emitted } => {
                    let s = pick_seq(hi, seq);
                    let want = match req {
                        ReqPick::None => None,
                        ReqPick::New => Some(s),
                        ReqPick::Same => Some(o.last_req.unwrap_or(s)),
                        ReqPick::Older => Some(o.last_req.unwrap_or(s).wrapping_sub(2).max(1)),
                    };
                    let em = emitted.then_some(GATE_STAMP);
                    // the oracle (G1-G3)
                    let fresh = o.last_seq.is_none_or(|l| newer(s, l));
                    if fresh {
                        o.last_seq = Some(s);
                        o.ticks_coalesced += 2;
                        if let Some(r) = want {
                            if o.last_req.is_none_or(|l| newer(r, l)) {
                                o.last_req = Some(r);
                                if o.pending.is_some() {
                                    o.coalesced += 1;
                                }
                                let at = match em {
                                    Some(st) => st,
                                    None => {
                                        o.deferred += 1;
                                        GATE_LAST_TICK + GATE_GUARD
                                    }
                                };
                                o.pending = Some((r, at));
                            } else {
                                o.stale_credits += 1;
                            }
                        }
                    } else {
                        o.stale += 1;
                        if want.is_some() {
                            o.stale_credits += 1;
                        }
                    }
                    let got = g.on_tick(s, 2, want, em, Some(GATE_LAST_TICK), GATE_GUARD);
                    assert_eq!(got, fresh, "{op:?} seq {s}");
                    if fresh {
                        hi2 = Some(hi.map_or(s, |h| if newer(s, h) { s } else { h }));
                    }
                }
                GateOp::Poll(t) => {
                    let want = match o.pending {
                        Some((r, at)) if t >= at => {
                            o.pending = None;
                            o.granted += 1;
                            Some(r)
                        }
                        _ => None,
                    };
                    assert_eq!(g.poll(t), want, "{op:?}");
                }
            }
            *n += 1;
            let c = g.c;
            assert_eq!(
                (c.stale, c.stale_credits, c.credits_coalesced, c.credits_deferred, c.granted, c.ticks_coalesced),
                (o.stale, o.stale_credits, o.coalesced, o.deferred, o.granted, o.ticks_coalesced),
                "{op:?}"
            );
            assert_eq!(g.pending(), o.pending, "{op:?}");
            go(ops, depth - 1, &g, &o, hi2, n);
        }
    }
    let mut n = 0;
    go(&ops, 4, &CreditGate::new(), &GateOracle::default(), None, &mut n);
    eprintln!("lockstep_exhaustive gate oracle: {n} operations, {} letters, depth 4", ops.len());
}

#[derive(Clone, Copy, Debug)]
enum ReqOp {
    Tick(SeqPick, bool),
    Spend,
    Idle(bool),
    Adv(u64),
}

#[derive(Clone, Default, Debug)]
struct ReqOracle {
    last_seq: Option<u32>,
    open: Option<u32>,
    since: u32,
    last_answer: Option<u64>,
    wants: u64,
    replaced: u64,
    frames: u64,
    no_change: u64,
    stale: u64,
    ticks: u64,
}

/// ★ The request book against its specification (R1-R4 and the NO_CHANGE pacing), every sequence of up
/// to 5 operations, the clock moving by 1 ms and 20 ms.
#[test]
fn lockstep_exhaustive_the_request_book_against_an_oracle() {
    use crate::tick::seq_newer as newer;
    let mut ops: Vec<ReqOp> = Vec::new();
    for s in [SeqPick::Next, SeqPick::Dup, SeqPick::Old, SeqPick::Skip5] {
        for w in [false, true] {
            ops.push(ReqOp::Tick(s, w));
        }
    }
    ops.extend([ReqOp::Spend, ReqOp::Idle(true), ReqOp::Idle(false), ReqOp::Adv(1), ReqOp::Adv(20)]);
    let gap = (P / 2_000_000).max(1);
    fn go(
        ops: &[ReqOp],
        depth: usize,
        r: &Requests,
        o: &ReqOracle,
        hi: Option<u32>,
        now: u64,
        gap: u64,
        n: &mut u64,
    ) {
        if depth == 0 {
            return;
        }
        for &op in ops {
            let (mut r, mut o, mut hi2, mut now2) = (r.clone(), o.clone(), hi, now);
            match op {
                ReqOp::Tick(p, want) => {
                    let s = pick_seq(hi, p);
                    let t = TickPkt {
                        tick_seq: s,
                        refresh_ns: P as u32,
                        want,
                        clock_only: false,
                        present_ns: 1,
                    };
                    let fresh = o.last_seq.is_none_or(|l| newer(s, l));
                    let v = r.on_tick(&t);
                    if fresh {
                        o.last_seq = Some(s);
                        o.ticks += 1;
                        o.since = o.since.saturating_add(1);
                        if want {
                            o.wants += 1;
                            if o.open.is_some() {
                                o.replaced += 1;
                            }
                            o.open = Some(s);
                            o.since = 0;
                        }
                        hi2 = Some(hi.map_or(s, |h| if newer(s, h) { s } else { h }));
                    } else {
                        o.stale += 1;
                    }
                    assert_eq!(matches!(v, TickVerdict::Fresh { .. }), fresh, "{op:?}");
                }
                ReqOp::Spend => {
                    let got = r.spend(now);
                    let want = o.open.take();
                    if want.is_some() {
                        o.frames += 1;
                        o.last_answer = Some(now);
                    }
                    assert_eq!(got, want, "{op:?}");
                }
                ReqOp::Idle(nothing) => {
                    let got = r.idle_request(nothing, now);
                    let want = o.open.filter(|_| {
                        nothing && o.since >= 1 && o.last_answer.is_none_or(|t| now >= t + gap)
                    });
                    assert_eq!(got, want, "{op:?} at {now}");
                    if got.is_some() {
                        r.no_change_done(now);
                        o.open = None;
                        o.no_change += 1;
                        o.last_answer = Some(now);
                    }
                }
                ReqOp::Adv(d) => now2 += d,
            }
            *n += 1;
            let c = r.c;
            assert_eq!(
                (c.ticks, c.ticks_stale, c.want_ticks, c.credits_coalesced, c.frames_on_credit, c.no_change_sent),
                (o.ticks, o.stale, o.wants, o.replaced, o.frames, o.no_change),
                "{op:?}"
            );
            assert_eq!(r.open(), o.open, "{op:?}");
            go(ops, depth - 1, &r, &o, hi2, now2, gap, n);
        }
    }
    let mut n = 0;
    go(&ops, 5, &Requests::new(), &ReqOracle::default(), None, 100, gap, &mut n);
    eprintln!("lockstep_exhaustive request-book oracle: {n} operations, {} letters, depth 5", ops.len());
}
