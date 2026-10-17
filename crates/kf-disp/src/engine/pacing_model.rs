//! ★ The exclusive-fullscreen present loop as a GPU-free MODEL (`docs/design/V3_DISPLAY_PACING_20261017.md`
//! §2 and §6): a guest that renders a frame, flips it, waits for the flip's completion and renders
//! again, driven against the real [`Engine`], the real [`Pacer`]/[`crate::tick::TickSource`] and
//! [`Mailbox`], and a model of the display worker's completion rule, on a simulated clock. Nothing
//! here measures hardware: it is the arithmetic of the code, run.
//!
//! The first version of this file (commit `0780de42e`, before the gate was removed) ran the same loop
//! against the D1 gate: its table is recorded in the design doc §2 as MEASURED on that code.

use super::tests::{CORE, Ring, WIN, engine, fl, lit, m, ma};
use super::*;
use crate::class::put;
use crate::pace::{Mailbox, Pacer, flip_completion_need};
use std::collections::VecDeque;

const MS: u64 = 1_000_000;
/// The model's clock step.
const DT: u64 = 25_000;
/// One 60 Hz tick (the Pacer's `1e9 / 60`).
const P60: u64 = 16_666_666;

/// What the guest's flips ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// Every flip immediate (async / tearing).
    Immediate,
    /// Every flip non-tearing (vsync).
    Vsync,
    /// Immediate, vsync, immediate, ...
    Alternate,
    /// Immediate, but every n-th flip also carries the CORE (a modeset-class update).
    CoreEvery(u32),
}

/// The display worker's rule for completions and copies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Worker {
    /// Before 2026-10-17: a latch of the shown window wants a copy NOW and every item behind it waits
    /// for copy `started + 1` — its own.
    Legacy,
    /// §AK: [`Mailbox`] copies once per tick; a flip's items wait only for the copy in flight.
    Mailbox,
    /// MUTANT of [`Worker::Mailbox`] for the buffer-reuse test: the completion ignores the copy in
    /// flight ([`flip_completion_need`] replaced by "nothing").
    MailboxIgnoringInflight,
}

/// The model's parameters.
#[derive(Clone, Copy, Debug)]
pub(super) struct Cfg {
    /// Swap-chain buffers (2 = double buffering).
    pub buffers: usize,
    /// GPU render time of one frame.
    pub render_ns: u64,
    /// One scanout copy (the worker's `scanout_avg_us` was about 1.8 ms on the Vast boxes).
    pub copy_ns: u64,
    /// From a completion being published to the guest thread acting on it (interrupt + wake).
    pub wake_ns: u64,
    /// The flip kind.
    pub kind: Kind,
    /// The tick period.
    pub period_ns: u64,
    /// The head's first tick, as an offset from the start.
    pub phase_ns: u64,
    /// The flip is submitted when its frame's render STARTS and carries a semaphore acquire that
    /// holds when the render ends (a pipelined driver), instead of after the render.
    pub acquire: bool,
    /// The worker's rule.
    pub worker: Worker,
}

impl Cfg {
    pub(super) fn new(buffers: usize, render_ms: f64, kind: Kind) -> Cfg {
        Cfg {
            buffers,
            render_ns: (render_ms * MS as f64) as u64,
            copy_ns: 2 * MS,
            wake_ns: MS / 2,
            kind,
            period_ns: P60,
            phase_ns: P60,
            acquire: false,
            worker: Worker::Mailbox,
        }
    }
}

/// A broker that sends tick events: every `period_ns` from `from_ns` up to `until_ns`.
#[derive(Clone, Copy, Debug)]
pub(super) struct Broker {
    pub from_ns: u64,
    pub until_ns: u64,
    pub period_ns: u64,
}

/// What a run measured, over the window after its first second.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Report {
    /// Flips whose completion the guest saw, per second.
    pub flips_per_s: f64,
    /// Latches per second (`kms`).
    pub presents_per_s: f64,
    /// Tearing latches per second (`tear`).
    pub tear_per_s: f64,
    /// Scanout copies started per second (`copies`).
    pub copies_per_s: f64,
    /// Ticks per second.
    pub ticks_per_s: f64,
    /// Latches that held the core, per second (`core_imm`).
    pub core_imm_per_s: f64,
    /// The longest a flip waited from submission to the guest seeing its completion.
    pub worst_flip_ms: f64,
    /// Latched frames that were replaced before a copy, per second.
    pub skipped_per_s: f64,
    /// Releases of a buffer a copy in flight was still reading (must be 0).
    pub violations: u64,
    /// The longest silence between two completions the guest saw, in the measured window.
    pub max_gap_ms: f64,
    /// Ticks the guest-visible vblank counter went backwards or repeated (must be 0).
    pub bad_seq: u64,
    /// Handoffs of the tick source: to the broker, back to the free clock.
    pub handoffs: (u64, u64),
    /// The smallest spacing between two consecutive ticks, ms.
    pub min_tick_gap_ms: f64,
}

/// A ring buffer the model appends methods to (NVKMS's JUMP-to-0 wrap at the end).
struct Writer {
    ring: Vec<u32>,
    off: usize,
}

impl Writer {
    fn new() -> Writer {
        Writer {
            ring: vec![0; 1024],
            off: 0,
        }
    }

    /// Continue after the methods already in `r`.
    fn after(r: &Ring) -> Writer {
        let mut w = Writer::new();
        w.ring[..r.words.len()].copy_from_slice(&r.words);
        w.off = r.words.len() * 4;
        w
    }

    /// Append `methods`; returns the ring's bytes and the new PUT.
    fn push(&mut self, methods: &[(u32, u32)]) -> (Vec<u8>, u32) {
        if self.off + methods.len() * 8 > 4040 {
            self.ring[self.off / 4] = 1 << 29; // JUMP 0
            self.off = 0;
        }
        for &(method, data) in methods {
            self.ring[self.off / 4] = (1 << 18) | method;
            self.ring[self.off / 4 + 1] = data;
            self.off += 8;
        }
        (
            self.ring.iter().flat_map(|w| w.to_le_bytes()).collect(),
            self.off as u32,
        )
    }
}

/// The worker's copy and completion bookkeeping (`kf-qemu` `ScanState`), per [`Worker`] rule.
struct Model {
    rule: Worker,
    mailbox: Mailbox,
    started: u64,
    done: u64,
    barrier: u64,
    want: bool,
    /// `(copy number, completes at, the flip index it reads)`.
    inflight: Option<(u64, u64, Option<u64>)>,
    /// `(needs copy, the latch index if it is a window latch)`.
    queue: VecDeque<(u64, Option<u64>)>,
    copies: u64,
    latches: u64,
    violations: u64,
}

impl Model {
    fn new(rule: Worker) -> Model {
        Model {
            rule,
            mailbox: Mailbox::default(),
            started: 0,
            done: 0,
            barrier: 0,
            want: false,
            inflight: None,
            queue: VecDeque::new(),
            copies: 0,
            latches: 0,
            violations: 0,
        }
    }

    /// The step's effects, queued in order with the barrier the rule gives them.
    fn effects(&mut self, step: &Step) {
        for e in &step.effects {
            let idx = matches!(e, Effect::Latched { .. }).then(|| {
                self.latches += 1;
                self.latches - 1
            });
            if idx.is_some() {
                match self.rule {
                    Worker::Legacy => {
                        self.barrier = self.started + 1;
                        self.want = true;
                    }
                    Worker::Mailbox => {
                        self.mailbox.latch();
                        self.barrier =
                            flip_completion_need(self.barrier, self.inflight.map(|f| f.0));
                    }
                    Worker::MailboxIgnoringInflight => self.mailbox.latch(),
                }
            }
            self.queue.push_back((self.barrier, idx));
        }
    }

    fn tick(&mut self) {
        self.mailbox.tick();
    }

    /// Start a copy if one is due.
    fn pass(&mut self, now: u64, copy_ns: u64) {
        let start = match self.rule {
            Worker::Legacy => self.want && self.inflight.is_none(),
            _ => self.mailbox.due(self.inflight.is_some()),
        };
        if start {
            assert!(
                self.inflight.is_none() || self.rule == Worker::MailboxIgnoringInflight,
                "one copy at a time, on the plane's own stream"
            );
            self.started += 1;
            self.copies += 1;
            self.want = false;
            self.mailbox.copied();
            let reads = self.latches.checked_sub(1);
            self.inflight = Some((self.started, now + copy_ns, reads));
        }
    }

    /// The copy completes at `now`.
    fn complete(&mut self, now: u64, copy_ns: u64) -> u32 {
        if let Some((n, at, _)) = self.inflight
            && now >= at
        {
            self.inflight = None;
            self.done = n;
        }
        let published = self.publish();
        self.pass(now, copy_ns);
        published
    }

    /// Publish what is no longer waiting: how many LATCHES went out. A latch releases the buffer of
    /// the flip before it: a copy still reading that buffer is a violation.
    fn publish(&mut self) -> u32 {
        let mut published = 0;
        while self
            .queue
            .front()
            .is_some_and(|(need, _)| *need <= self.done)
        {
            if let Some((_, Some(j))) = self.queue.pop_front() {
                published += 1;
                if j >= 1
                    && self
                        .inflight
                        .is_some_and(|(_, _, reads)| reads == Some(j - 1))
                {
                    self.violations += 1;
                }
            }
        }
        published
    }
}

/// Run the guest loop for `secs` seconds (the first second is warm-up), the head ticking from a
/// [`Pacer`] at 60 Hz and, if `broker` is given, a broker feeding it tick events.
pub(super) fn run(cfg: Cfg, broker: Option<Broker>, secs: u64) -> Report {
    let mut e = engine();
    let core_ring = lit(&mut e);
    let mut core_w = Writer::after(&core_ring);
    let mut win_w = Writer::new();
    let mut w = Model::new(cfg.worker);
    let mut pacer = Pacer::new(60);
    pacer.on_heads(
        &[HeadMode {
            head: 0,
            period_ns: cfg.period_ns,
            raster: (2200, 1125),
        }],
        cfg.phase_ns - cfg.period_ns,
    );

    let end = (secs + 1) * 1_000_000_000;
    let mut now = 0u64;
    let mut next_broker = broker.map(|b| b.from_ns);
    // the guest
    let mut k_render = 0u64; // the frame being (or next to be) rendered
    let mut render_until: Option<u64> = None;
    let mut ready: VecDeque<u64> = VecDeque::new();
    let mut outstanding: Option<u64> = None; // submitted at
    let mut completed = 0u64; // flips whose completion the guest saw
    let mut wakes: VecDeque<u64> = VecDeque::new();
    // counters at the start of the measured window
    let (mut base, mut base_completed, mut base_copies, mut base_ticks, mut base_skipped) =
        (PaceCounts::default(), 0u64, 0u64, 0u64, 0u64);
    let (mut ticks, mut worst, mut next_poll) = (0u64, 0u64, 0u64);
    let mut based = false;
    let (mut last_seq, mut bad_seq) = (0u64, 0u64);
    let (mut last_tick, mut min_gap) = (None::<u64>, u64::MAX);
    let (mut last_done, mut max_gap) = (None::<u64>, 0u64);

    while now < end {
        if !based && now >= 1_000_000_000 {
            based = true;
            base = e.pace[0];
            base_completed = completed;
            base_copies = w.copies;
            base_ticks = ticks;
            base_skipped = w.mailbox.skipped;
            last_done = Some(now);
        }
        // 1. the head's ticks: the free clock's, and the broker's events
        let mut got = pacer.due(now);
        if let (Some(b), Some(t)) = (broker, next_broker)
            && now >= t
        {
            next_broker = Some(t + b.period_ns).filter(|n| *n < b.until_ns);
            got.extend(pacer.broker_tick(0, now));
        }
        for t in got {
            if t.seq != last_seq + 1 {
                bad_seq += 1;
            }
            last_seq = t.seq;
            if let Some(l) = last_tick {
                min_gap = min_gap.min(now - l);
            }
            last_tick = Some(now);
            let sem = k_render;
            let s = e.vblank(0, &mut |a: &Acquire| sem >= a.value);
            w.effects(&s);
            w.tick();
            ticks += 1;
        }
        // 2. the worker's copy completes (and what was waiting for nothing goes out); the published
        // latches wake the guest
        for _ in 0..w.complete(now, cfg.copy_ns) {
            wakes.push_back(now + cfg.wake_ns);
        }
        // 3. the guest learns of a completion
        while wakes.front().is_some_and(|t| *t <= now) {
            wakes.pop_front();
            if let Some(at) = outstanding.take() {
                worst = worst.max(now - at);
            }
            completed += 1;
            if let Some(l) = last_done.as_mut() {
                max_gap = max_gap.max(now - *l);
                *l = now;
            }
        }
        // 3b. the acquire-only waits are re-read every 2 ms (the worker's poll)
        if e.acquire_pending() && now >= next_poll {
            next_poll = now + 2 * MS;
            let sem = k_render;
            let s = e.poll_acquires(&mut |a: &Acquire| sem >= a.value);
            w.effects(&s);
        }
        // 4. render
        if let Some(t) = render_until
            && now >= t
        {
            render_until = None;
            if !cfg.acquire {
                ready.push_back(k_render);
            }
            k_render += 1;
        }
        let free = k_render < cfg.buffers as u64 || completed + cfg.buffers as u64 >= k_render + 2;
        if render_until.is_none() && free {
            render_until = Some(now + cfg.render_ns);
            if cfg.acquire {
                ready.push_back(k_render);
            }
        }
        // 5. flip: one outstanding at a time (the driver programs flip N after N-1 finished)
        if outstanding.is_none()
            && let Some(k) = ready.pop_front()
        {
            let (mode, with_core) = match cfg.kind {
                Kind::Immediate => (1, false),
                Kind::Vsync => (0, false),
                Kind::Alternate => (u32::from(k % 2 == 0), false),
                Kind::CoreEvery(n) => (1, k % u64::from(n) == u64::from(n) - 1),
            };
            let mut methods = vec![
                (ma(WIN, "SET_CONTEXT_DMA_ISO", 0), 0x1_0001),
                (
                    m(WIN, "SET_PRESENT_CONTROL"),
                    put(0, fl(WIN, "SET_PRESENT_CONTROL_BEGIN_MODE"), mode),
                ),
            ];
            if matches!(cfg.kind, Kind::CoreEvery(_)) {
                // sticky assembly state: set it for the core flip, clear it for the others
                methods.push((
                    m(WIN, "SET_INTERLOCK_FLAGS"),
                    put(
                        0,
                        fl(WIN, "SET_INTERLOCK_FLAGS_INTERLOCK_WITH_CORE"),
                        u32::from(with_core),
                    ),
                ));
            }
            if cfg.acquire {
                methods.push((m(WIN, "SET_CONTEXT_DMA_ACQ_SEMAPHORE"), 0xcafe_0a00));
                methods.push((m(WIN, "SET_ACQ_SEMAPHORE_VALUE"), k as u32 + 1));
            }
            methods.push((m(WIN, "SET_WINDOW_INTERLOCK_FLAGS"), 0));
            methods.push((m(WIN, "UPDATE"), 0));
            let (bytes, p) = win_w.push(&methods);
            outstanding = Some(now);
            let sem = k_render;
            let mut s = e.step(1, &bytes, p, &mut |a: &Acquire| sem >= a.value);
            if with_core {
                let (bytes, p) = core_w.push(&[
                    (m(CORE, "SET_WINDOW_INTERLOCK_FLAGS"), 1),
                    (m(CORE, "UPDATE"), 0),
                ]);
                let s2 = e.step(0, &bytes, p, &mut |a: &Acquire| sem >= a.value);
                s.effects.extend(s2.effects);
            }
            w.effects(&s);
        }
        // anything that waits for no copy goes out in this pass
        for _ in 0..w.publish() {
            wakes.push_back(now + cfg.wake_ns);
        }
        w.pass(now, cfg.copy_ns);
        now += DT;
    }
    let sec = (end - 1_000_000_000) as f64 / 1e9;
    let p = e.pace[0];
    let src = pacer.source(0);
    Report {
        flips_per_s: (completed - base_completed) as f64 / sec,
        presents_per_s: (p.presents - base.presents) as f64 / sec,
        tear_per_s: (p.tearing - base.tearing) as f64 / sec,
        copies_per_s: (w.copies - base_copies) as f64 / sec,
        ticks_per_s: (ticks - base_ticks) as f64 / sec,
        core_imm_per_s: (p.core_imm - base.core_imm) as f64 / sec,
        worst_flip_ms: worst as f64 / 1e6,
        skipped_per_s: (w.mailbox.skipped - base_skipped) as f64 / sec,
        violations: w.violations,
        max_gap_ms: max_gap as f64 / 1e6,
        bad_seq,
        handoffs: src.map_or((0, 0), |s| s.handoffs()),
        min_tick_gap_ms: if min_gap == u64::MAX {
            0.0
        } else {
            min_gap as f64 / 1e6
        },
    }
}

fn near(x: f64, y: f64, tol: f64) -> bool {
    (x - y).abs() <= tol
}

/// ★ TASK 1 (the reproduction), the D1 gate's side — NOT runnable on this code, kept as the record: on
/// `ec42327f0` / `0780de42e` (gate ON, 60 Hz tick, 2 ms copy, 0.5 ms wake), flips completed per second
/// against the render time R (MEASURED by this model before the gate was removed):
///
/// ```text
/// 2 buffers, immediate           gate ON : R 1..14 = 60.0   R16 = 54.0  R18 = 49.0  R20 = 44.5  R25 = 36.2  R33 = 28.0
/// 2 buffers, immediate           gate OFF: R 1 = 285.8 R 2 = 222.2 R 4 = 153.8 R 8 = 95.2 R12 = 68.8 R14 = 60.8
/// 3 buffers, immediate           gate ON : R 1..16 = 60.0  R18 = 55.5  R20 = 50.0  R25 = 40.0  R33 = 30.2
/// 3 buffers, immediate           gate OFF: R 1 = 400.0 R 2 = 400.0 R 4 = 250.0 R 8 = 125.0 R12 = 83.2
/// 2 buffers, vsync               either  : R 1..14 = 60.0   R16..25 = 30.0   R33 = 20.0
/// 2 buffers, alternate imm/vsync gate ON : R 1..14 = 60.0 (= 30 frames/s)   R16..20 = 40.0  R25 = 30.0
/// 2 buffers, alternate imm/vsync gate OFF: R 1..4 = 120.0 (= 60 frames/s)   R8..14 = 60.0
/// 2 buffers, core every 4        gate ON : R 1..8 = 80.0  (60 + 20 core)    R12 = 68.5  R14 = 60.8
/// 2 buffers, immediate, acquire  gate ON : R 1..14 = 60.0   R16..25 = 30.0 (the acquire is read only at a tick)   R33 = 20.0
/// 2 buffers, immediate, acquire  gate OFF: R 1 = 221.0 R 4 = 153.2 R 8 = 95.0
/// copy sweep, 2 buffers, immediate, R = 4, gate ON: copy 1..12 ms = 60.0 (copies 60);  14 = 54.0;  18 = 44.5;  25 = 33.8;  33 = 26.8 (copies = flips)
/// ```
///
/// The same loop on the code of this commit is what the tests below run.
#[test]
fn the_old_gate_numbers_are_recorded_here() {}

/// ★ Task 1 on the NEW code (§AK): the gate is gone, the display still ticks at 60. One immediate
/// flip per frame, 2 buffers, R = 4 ms: at least 200 flips a second; the copies stay at one per tick;
/// no released buffer is ever read by a copy.
#[test]
fn immediate_flips_run_at_the_render_rate_while_the_display_ticks_at_60() {
    let mut line = String::new();
    for buffers in [2usize, 3] {
        for r in [1.0, 2.0, 4.0, 8.0, 12.0] {
            let rep = run(Cfg::new(buffers, r, Kind::Immediate), None, 4);
            line += &format!(
                "\n  {buffers} buffers R={r:>4}: flips {:>6.1}/s copies {:>5.1}/s ticks {:>5.1}/s skipped {:>6.1}/s violations {}",
                rep.flips_per_s,
                rep.copies_per_s,
                rep.ticks_per_s,
                rep.skipped_per_s,
                rep.violations
            );
            assert_eq!(rep.violations, 0, "{rep:?}");
            assert!(rep.copies_per_s <= rep.ticks_per_s + 0.6, "{rep:?}");
            assert!(near(rep.ticks_per_s, 60.0, 0.6), "{rep:?}");
        }
    }
    eprintln!("{line}");
    let rep = run(Cfg::new(2, 4.0, Kind::Immediate), None, 4);
    assert!(rep.flips_per_s >= 200.0, "R=4, 2 buffers: {rep:?}");
    assert!(
        rep.skipped_per_s > 100.0,
        "newest wins, the rest are skipped: {rep:?}"
    );
    // 3 buffers: the GPU can run ahead
    let rep3 = run(Cfg::new(3, 4.0, Kind::Immediate), None, 4);
    assert!(rep3.flips_per_s > rep.flips_per_s, "{rep3:?}");
}

/// ★ A vsync flip still completes at its tick: one per tick, and 30 a second when the render outlasts
/// one — what a non-tearing flip is. (Owner decision (a): vsync flips complete at the next vblank.)
#[test]
fn vsync_flips_still_complete_at_the_tick() {
    let r = run(Cfg::new(2, 4.0, Kind::Vsync), None, 4);
    assert!(near(r.flips_per_s, 60.0, 0.6), "{r:?}");
    assert!(near(r.copies_per_s, 60.0, 0.6), "{r:?}");
    assert_eq!(r.violations, 0);
    let slow = run(Cfg::new(2, 20.0, Kind::Vsync), None, 4);
    assert!(near(slow.flips_per_s, 30.0, 0.6), "{slow:?}");
}

/// ★ Two flips per frame (one immediate, one vsync — the only structure the owner's counters fit): the
/// immediate one no longer waits for a tick, the vsync one does: 60 frames a second at R = 4 ms where
/// the gate gave 30. The ceiling is the vsync flip: this model does not promise more than 60.
#[test]
fn an_alternating_loop_is_bounded_by_its_vsync_flips() {
    let r = run(Cfg::new(2, 4.0, Kind::Alternate), None, 4);
    eprintln!("alternate: {r:?}");
    assert!(near(r.flips_per_s / 2.0, 60.0, 1.0), "frames/s {r:?}");
    assert_eq!(r.violations, 0);
}

/// A pipelined driver (the flip is submitted at the render's start with a semaphore acquire): the
/// acquire of an immediate flip is read every 2 ms, so a long render no longer costs a tick.
#[test]
fn an_acquire_costs_two_milliseconds_not_a_tick() {
    let mut cfg = Cfg::new(2, 18.0, Kind::Immediate);
    cfg.acquire = true;
    let r = run(cfg, None, 4);
    eprintln!("acquire R=18: {r:?}");
    assert!(r.flips_per_s > 45.0, "(the gate gave 30.0 here) {r:?}");
    assert_eq!(r.violations, 0);
}

/// A modeset-class update every 4th flip latches with the core at once: the loop is not slowed.
#[test]
fn a_core_holding_flip_changes_nothing() {
    let r = run(Cfg::new(2, 4.0, Kind::CoreEvery(4)), None, 4);
    assert!(r.flips_per_s >= 200.0, "{r:?}");
    assert!(r.core_imm_per_s > 30.0, "{r:?}");
    assert_eq!(r.violations, 0);
}

/// ★ THE BUFFER-REUSE RULE. A flip's completion releases the buffer it replaced; a copy still reading
/// that buffer must finish first. With the rule, no release ever overlaps a copy of the released
/// buffer, across copy times from instant to longer than the tick; without it (the mutant worker:
/// completion ignores the copy in flight) the model finds the overlap.
#[test]
fn a_flip_never_completes_under_a_copy_of_the_buffer_it_replaces() {
    for copy_ms in [0u64, 1, 2, 5, 8, 12, 16, 20, 33] {
        for (buffers, r) in [(2usize, 1.0), (2, 4.0), (3, 2.0), (3, 8.0)] {
            let mut cfg = Cfg::new(buffers, r, Kind::Immediate);
            cfg.copy_ns = copy_ms * MS;
            let rep = run(cfg, None, 3);
            assert_eq!(
                rep.violations, 0,
                "copy {copy_ms} ms, {buffers} buffers, R {r}: {rep:?}"
            );
            assert!(rep.flips_per_s > 10.0, "alive: {rep:?}");
        }
    }
    let mut cfg = Cfg::new(2, 1.0, Kind::Immediate);
    cfg.copy_ns = 8 * MS;
    cfg.worker = Worker::MailboxIgnoringInflight;
    let rep = run(cfg, None, 3);
    assert!(rep.violations > 50, "the mutant must be caught: {rep:?}");
    // ... and the completion waits for the COPY (milliseconds), never for a tick
    let mut cfg = Cfg::new(2, 1.0, Kind::Immediate);
    cfg.copy_ns = 8 * MS;
    let rep = run(cfg, None, 3);
    assert!(
        rep.worst_flip_ms < 12.0,
        "a flip waited past the copy: {rep:?}"
    );
}

/// What the old worker rule gives once the gate is gone and nothing else changes: every latch wants
/// its own copy and its completion waits for it, so the copies are no longer bounded by the tick (the
/// reason the mailbox exists).
#[test]
fn the_legacy_worker_without_the_gate_copies_every_flip() {
    let mut cfg = Cfg::new(2, 4.0, Kind::Immediate);
    cfg.worker = Worker::Legacy;
    let r = run(cfg, None, 4);
    assert!(r.copies_per_s > 120.0, "{r:?}");
}

/// ★ A broker that is silent, then speaks, then dies: the guest's flips never stop. With vsync flips
/// (which need ticks) the longest silence between completions stays under 100 ms through a hole in the
/// broker's events, the vblank counter never repeats, and no two ticks are closer than half a period.
#[test]
fn a_dead_broker_never_freezes_the_guest() {
    let b = Broker {
        from_ns: 1_000_000_000 + 5 * MS,
        until_ns: 2_000_000_000,
        period_ns: P60,
    };
    let r = run(Cfg::new(2, 4.0, Kind::Vsync), Some(b), 4);
    eprintln!("broker 1.0..2.0 s then dead: {r:?}");
    assert!(r.max_gap_ms < 100.0, "{r:?}");
    assert_eq!(r.bad_seq, 0, "{r:?}");
    assert_eq!(r.handoffs, (1, 1), "{r:?}");
    assert!(near(r.flips_per_s, 60.0, 2.0), "{r:?}");
    assert!(r.min_tick_gap_ms >= 0.5 * 16.666, "no double tick: {r:?}");
}

/// A broker whose events interleave badly with the free clock (period 15.9 ms, drifting through every
/// phase against the clock): never two ticks closer than half a period, the counter exact.
#[test]
fn a_broker_drifting_through_the_clock_never_double_ticks() {
    let b = Broker {
        from_ns: 1_000_000_000 + 3 * MS,
        until_ns: 3_000_000_000,
        period_ns: 15_900_000,
    };
    let r = run(Cfg::new(2, 4.0, Kind::Vsync), Some(b), 4);
    assert_eq!(r.bad_seq, 0, "{r:?}");
    assert!(r.min_tick_gap_ms >= 0.5 * 16.666 - 0.1, "{r:?}");
    assert!(r.max_gap_ms < 100.0, "{r:?}");
}
