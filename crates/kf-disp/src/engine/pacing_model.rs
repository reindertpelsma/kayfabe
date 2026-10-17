//! ★ The exclusive-fullscreen present loop as a GPU-free MODEL (`docs/design/V3_DISPLAY_PACING_20261017.md`
//! §2): a guest that renders a frame, flips it, waits for the flip's completion and renders again,
//! driven against the real [`Engine`] and a model of the display worker's copy/completion rule, on a
//! simulated clock. Nothing here is a measurement of hardware: it is the arithmetic of the code, run.

use super::tests::{CORE, Ring, WIN, all_ok, engine, fl, lit, m, ma};
use super::*;
use crate::class::put;
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
    /// The flip is submitted when its frame's render STARTS and carries a semaphore acquire that holds
    /// when the render ends (a pipelined driver), instead of after the render.
    pub acquire: bool,
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
        }
    }
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
    /// Tearing flips the gate parked per second (`held`; the gate is gone after phase 1).
    pub held_per_s: f64,
    /// Latches that held the core, per second (`core_imm`).
    pub core_imm_per_s: f64,
    /// The longest a flip waited from submission to the guest seeing its completion.
    pub worst_flip_ms: f64,
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

/// One head's scanout-copy bookkeeping as the worker is today (`kf-qemu` `ScanState`): a latch of the
/// console window wants a copy NOW and every item behind it waits for copy `started + 1`.
#[derive(Default)]
struct LegacyWorker {
    started: u64,
    done: u64,
    barrier: u64,
    want: bool,
    inflight: Option<(u64, u64)>,
    queue: VecDeque<(u64, bool)>,
    copies: u64,
}

impl LegacyWorker {
    fn effects(&mut self, step: &Step) {
        for e in &step.effects {
            let latched = matches!(e, Effect::Latched { .. });
            if latched {
                self.barrier = self.started + 1;
                self.want = true;
            }
            self.queue.push_back((self.barrier, latched));
        }
    }

    fn pass(&mut self, now: u64, copy_ns: u64) {
        if self.want && self.inflight.is_none() {
            self.started += 1;
            self.copies += 1;
            self.want = false;
            self.inflight = Some((self.started, now + copy_ns));
        }
    }

    /// The copy completes at `now`: how many LATCHES are published by it.
    fn complete(&mut self, now: u64, copy_ns: u64) -> u32 {
        let Some((n, at)) = self.inflight else {
            return 0;
        };
        if now < at {
            return 0;
        }
        self.inflight = None;
        self.done = n;
        let mut published = 0;
        while self.queue.front().is_some_and(|(need, _)| *need <= self.done) {
            published += u32::from(self.queue.pop_front().is_some_and(|(_, l)| l));
        }
        self.pass(now, copy_ns);
        published
    }
}

/// Run the guest loop for `secs` seconds (the first second is warm-up) against the engine with the
/// D1 gate `gate`.
pub(super) fn run(cfg: Cfg, gate: bool, secs: u64) -> Report {
    let mut e = engine();
    e.tear_gate = gate;
    let core_ring = lit(&mut e);
    let mut core_w = Writer::after(&core_ring);
    let mut win_w = Writer::new();
    let mut worker = LegacyWorker::default();

    let end = (secs + 1) * 1_000_000_000;
    let (mut now, mut next_tick) = (0u64, cfg.phase_ns);
    // the guest
    let mut k_render = 0u64; // the frame being (or next to be) rendered
    let mut render_until: Option<u64> = None;
    let mut ready: VecDeque<u64> = VecDeque::new();
    let mut outstanding: Option<(u64, u64)> = None; // (frame, submitted at)
    let mut completed = 0u64; // flips whose completion the guest saw
    let mut wakes: VecDeque<u64> = VecDeque::new();
    // the window's counters at the start of the measured window
    let (mut base, mut base_completed, mut base_copies, mut base_ticks) =
        (PaceCounts::default(), 0u64, 0u64, 0u64);
    let mut ticks = 0u64;
    let mut worst = 0u64;
    let mut next_poll = 0u64;
    let mut based = false;

    while now < end {
        if !based && now >= 1_000_000_000 {
            based = true;
            base = e.pace[0];
            base_completed = completed;
            base_copies = worker.copies;
            base_ticks = ticks;
        }
        // 1. the head's tick
        if now >= next_tick {
            let sem = k_render;
            let s = e.vblank(0, &mut |a: &Acquire| sem >= a.value);
            worker.effects(&s);
            ticks += 1;
            next_tick += cfg.period_ns;
        }
        // 2. the worker's copy completes; its published latches wake the guest
        for _ in 0..worker.complete(now, cfg.copy_ns) {
            wakes.push_back(now + cfg.wake_ns);
        }
        // 3. the guest learns of a completion
        while wakes.front().is_some_and(|t| *t <= now) {
            wakes.pop_front();
            if let Some((_, at)) = outstanding.take() {
                worst = worst.max(now - at);
            }
            completed += 1;
        }
        // 3b. the acquire-only waits are re-read every 2 ms (the worker's poll)
        if e.acquire_pending() && now >= next_poll {
            next_poll = now + 2 * MS;
            let sem = k_render;
            let s = e.poll_acquires(&mut |a: &Acquire| sem >= a.value);
            worker.effects(&s);
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
            outstanding = Some((k, now));
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
            worker.effects(&s);
        }
        worker.pass(now, cfg.copy_ns);
        now += DT;
    }
    let sec = (end - 1_000_000_000) as f64 / 1e9;
    let p = e.pace[0];
    Report {
        flips_per_s: (completed - base_completed) as f64 / sec,
        presents_per_s: (p.presents - base.presents) as f64 / sec,
        tear_per_s: (p.tearing - base.tearing) as f64 / sec,
        copies_per_s: (worker.copies - base_copies) as f64 / sec,
        ticks_per_s: (ticks - base_ticks) as f64 / sec,
        held_per_s: (p.tear_held - base.tear_held) as f64 / sec,
        core_imm_per_s: (p.core_imm - base.core_imm) as f64 / sec,
        worst_flip_ms: worst as f64 / 1e6,
    }
}

/// ★ TASK 1 (the reproduction): with the D1 gate ON at a 60 Hz tick, flips completed per second
/// against the render time R, for the variants. Printed (run with `--nocapture`); the asserts are the
/// findings recorded in `V3_DISPLAY_PACING_20261017.md` §2, so they cannot rot unnoticed.
#[test]
fn the_gate_on_model_table() {
    let rs = [1.0, 2.0, 4.0, 8.0, 12.0, 14.0, 16.0, 18.0, 20.0, 25.0, 33.0];
    for (name, buffers, kind, acquire) in [
        ("2 buffers, immediate", 2, Kind::Immediate, false),
        ("3 buffers, immediate", 3, Kind::Immediate, false),
        ("2 buffers, vsync", 2, Kind::Vsync, false),
        ("2 buffers, alternate imm/vsync", 2, Kind::Alternate, false),
        ("2 buffers, core every 4", 2, Kind::CoreEvery(4), false),
        ("2 buffers, immediate, acquire", 2, Kind::Immediate, true),
        ("3 buffers, immediate, acquire", 3, Kind::Immediate, true),
    ] {
        for gate in [true, false] {
            let mut line = format!("{name:32} gate={gate:5}:");
            for r in rs {
                let mut cfg = Cfg::new(buffers, r, kind);
                cfg.acquire = acquire;
                let rep = run(cfg, gate, 4);
                line += &format!(" R{r:>4}={:>6.1}", rep.flips_per_s);
            }
            eprintln!("{line}");
        }
    }
    // FINDING 1: one immediate flip per frame, gate ON: exactly one flip per tick for every R the
    // tick can hide (R + 2.5 ms of copy and wake inside 16.7 ms) — 60, NOT 30
    for r in [1.0, 2.0, 4.0, 8.0, 12.0] {
        let rep = run(Cfg::new(2, r, Kind::Immediate), true, 4);
        assert!((rep.flips_per_s - 60.0).abs() < 0.6, "R={r}: {rep:?}");
        assert!((rep.held_per_s - 60.0).abs() < 0.6, "every flip is parked: {rep:?}");
    }
    // ... and the gate is what holds it: off, the same loop runs at the render + copy + wake time
    let off = run(Cfg::new(2, 4.0, Kind::Immediate), false, 4);
    assert!(off.flips_per_s > 140.0, "{off:?}");
}

/// ★ TASK 1: the counters of the owner's fullscreen line (`kms=59.9 tear=29.9 copies=59.9 tick=59.9`,
/// fps 30) are what the gate gives when each frame is TWO flips, one immediate and one not: 60
/// latches a second, 30 of them tearing, a copy for each, 30 frames. One immediate flip per frame
/// gives `tear` = `kms` = 60.
#[test]
fn the_gate_on_model_counters() {
    let one = run(Cfg::new(2, 4.0, Kind::Immediate), true, 4);
    eprintln!("one immediate flip per frame: {one:?}");
    assert!((one.tear_per_s - one.presents_per_s).abs() < 0.6);
    let two = run(Cfg::new(2, 4.0, Kind::Alternate), true, 4);
    eprintln!("an immediate and a vsync flip per frame: {two:?}");
    assert!((two.presents_per_s - 60.0).abs() < 0.6, "{two:?}");
    assert!((two.tear_per_s - 30.0).abs() < 0.6, "{two:?}");
    assert!((two.copies_per_s - 60.0).abs() < 0.6, "{two:?}");
    assert!((two.flips_per_s / 2.0 - 30.0).abs() < 0.6, "30 frames: {two:?}");
    // the copy-time sweep: a slow copy cuts the rate smoothly, and the copies fall with it, so
    // copies = 60 with 30 fps cannot be a slow copy
    for copy_ms in [1u64, 2, 5, 8, 12, 14, 16, 18, 25, 33] {
        let mut cfg = Cfg::new(2, 4.0, Kind::Immediate);
        cfg.copy_ns = copy_ms * MS;
        let r = run(cfg, true, 4);
        eprintln!(
            "2 buffers, immediate, R=4, copy={copy_ms:>2} ms: flips {:>5.1}/s copies {:>5.1}/s held {:>5.1}/s worst {:>5.1} ms",
            r.flips_per_s, r.copies_per_s, r.held_per_s, r.worst_flip_ms
        );
        assert!((r.copies_per_s - r.flips_per_s).abs() < 0.6, "one copy per flip: {r:?}");
    }
}
