//! ★★ **The lock-step SOAK** (`docs/design/V3_DISPLAY_PACING_20261017.md` §11.7; owner decisions §AK, §AL,
//! 2026-10-17): the whole flip path, GPU-free, deterministic, replayable by seed, no sleeping.
//!
//! WHAT RUNS (all REAL code unless marked):
//! - the display [`Engine`] — every flip is a real pushbuffer through `Engine::step`, every vblank a real
//!   `Engine::vblank` (as `pacing_model.rs` does);
//! - [`Pacer`] / [`crate::tick::TickSource`], [`CreditGate`], [`Mailbox`], [`flip_completion_need`]
//!   (through `pacing_model::Model`, the phase-1 worker model);
//! - `kf_broker::Relay<FakeLink>` over a real [`FrameRing`] (memfd slots), the real [`TickShare`] and the
//!   real wire encoding (`Pkt::tick_pkt`, `Cmd::decode`);
//! - a scripted, adversarial FAKE BROKER behind the link: it speaks the wire, keeps the lock-step
//!   reference (what the relay MAY send) and checks every command the relay emits.
//!
//! WHAT IS A MODEL: the worker's pass (`World::worker_pass`) is a copy of `kf-qemu` `display.rs` step 4 / 6
//! (`pacer.due` → `TickShare::snapshot` → `CreditGate::on_tick` → `pacer.broker_tick` → `gate.poll` →
//! `Mailbox::grant`; a FREE-clock tick grants by itself; copy on `Mailbox::due`; publish to the ring and
//! `relay.on_frame`). The real loop cannot run without a GPU: if `display.rs` changes, this copy must
//! follow. The guest is a model too (what Windows sends is inferred, `V3_DISPLAY_PACING` §2).
//!
//! INVARIANTS (checked after EVERY step; a failure prints the SEED and the last events):
//! no flip parked; no frame without a credit (every COMMIT answers the open request, once; an unsolicited
//! one only while the broker is silent); frames in flight <= 2 (a literal, not the constant); at most one
//! pending credit; no double tick inside the guard; the vblank counter +1 and never stalled for more
//! than 50 ms + two periods; the buffer-reuse rule; request numbers strictly increasing; at most
//! `READ_BATCH` packets per relay turn; bounded memory; every guest flip completes; the filled ring slot
//! is never a held or ready one; the relay's liveness is the 50 ms rule.
//!
//! Replay one seed: `KF_SOAK_SEED=<n> cargo test --release -p kf-disp --lib lockstep_soak_<name>`.
//! The long soak: `KF_SOAK_FLIPS=10000000 cargo test --release -p kf-disp --lib lockstep_soak_long -- --ignored`.
#![allow(clippy::too_many_lines, clippy::cognitive_complexity, clippy::type_complexity)]

use super::pacing_model::{Model, Worker, Writer};
use super::tests::{WIN, engine, fl, lit, m, ma};
use super::*;
use crate::class::put;
use crate::lockstep::CreditGate;
use crate::pace::Pacer;
use crate::tick::{Handoff, Source, seq_newer};
use kf_broker::conn::READ_BATCH;
use kf_broker::ticks::{PARTIAL_READ_MS, tick_live};
use kf_broker::wire::{
    CAP_DMABUF, CAP_FOCUS_EVENTS, CAP_RELEASE, CAP_TICK, CLIENT_TICK, CMD_ATTACH, CMD_CAPS,
    CMD_COMMIT, CMD_NO_CHANGE, CMD_SIZE, CMD_WINDOW, Cmd, EV_ABS, EV_DEVICE, EV_FOCUS, EV_FRAME,
    EV_HELLO, EV_POINTER, EV_RELEASE, EV_SURFACE, EV_TICK, FOURCC_XR24, PKT_SIZE, Pkt, TickPkt,
};
use kf_broker::{
    FrameGeom, FrameRing, Host, Input, Link, Recv, Relay, RelayConfig, Sent, SlotFds, TickShare,
};
use kf_linux_raw::{SharedRam, fd_inode};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

const MS: u64 = 1_000_000;
const P60: u64 = 16_666_666;
const ME: u32 = 1000;

// ------------------------------------------------------------------------------------------------
// the random source and the trace

/// xorshift64*: deterministic, tiny, replayable.
#[derive(Clone)]
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ 0xd1b5_4a32_d192_ed03 | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.below(hi.saturating_sub(lo) + 1)
    }
    /// `p` in permille.
    fn chance(&mut self, p: u32) -> bool {
        self.below(1000) < u64::from(p)
    }
}

/// One line of the trace, kept compact (formatted only on failure).
#[derive(Clone, Copy, Debug)]
enum Ev {
    Tick { seq: u32, want: bool },
    Stale { seq: u32 },
    Noise(u16),
    Flip { imm: bool },
    Done,
    Vblank { from: Source, seq: u64 },
    Handoff(Handoff),
    CopyStart,
    CopyDone,
    Publish { slot: usize },
    Commit { seq: u32 },
    NoChange { seq: u32 },
    Caps,
    Release,
    Act(&'static str),
    Connect,
    Lost,
    Grant { req: u32 },
}

struct Trace {
    ring: VecDeque<(u64, Ev)>,
}

impl Trace {
    fn push(&mut self, at: u64, e: Ev) {
        if self.ring.len() == 192 {
            self.ring.pop_front();
        }
        self.ring.push_back((at, e));
    }
}

// ------------------------------------------------------------------------------------------------
// the link, the host

#[derive(Default)]
struct Wire {
    up: bool,
    inbox: VecDeque<u8>,
    eof: bool,
    connects: u32,
    open: i32,
    sent: Vec<(Cmd, Option<u64>)>,
    /// The broker never reads: every send is `Full`.
    never_reads: bool,
    /// `Full` results returned since the harness last reset it: a relay that retries a full send in
    /// a loop trips the panic below (the send-nonblocking rule).
    full_run: u32,
    fulls: u64,
    /// The next `recv` reports a descriptor attached to the stream (`MSG_CTRUNC`).
    fd_drop: bool,
    /// The kernel socket buffer, in bytes: what the broker cannot push is dropped by IT (a real broker
    /// coalesces; either way kayfabe's memory is the socket's, not unbounded).
    inbox_cap: usize,
    sock_full_drops: u64,
}

impl Wire {
    fn push(&mut self, bytes: &[u8]) -> bool {
        if self.inbox.len() + bytes.len() > self.inbox_cap {
            self.sock_full_drops += 1;
            return false;
        }
        self.inbox.extend(bytes);
        true
    }
}

#[derive(Clone)]
struct FakeLink(Rc<RefCell<Wire>>);

struct Sock(Rc<RefCell<Wire>>);

impl Drop for Sock {
    fn drop(&mut self) {
        self.0.borrow_mut().open -= 1;
    }
}

impl Link for FakeLink {
    type Sock = Sock;
    fn connect(&mut self, _p: &Path) -> Result<Sock, String> {
        let mut w = self.0.borrow_mut();
        if !w.up {
            return Err("connection refused".into());
        }
        w.connects += 1;
        w.open += 1;
        w.inbox.clear();
        w.eof = false;
        w.fd_drop = false;
        Ok(Sock(self.0.clone()))
    }
    fn fd(&self, _s: &Sock) -> i32 {
        77
    }
    fn peer_uid(&mut self, _s: &Sock) -> Result<u32, String> {
        Ok(ME)
    }
    fn effective_uid(&mut self) -> u32 {
        ME
    }
    fn send(&mut self, _s: &Sock, rec: &[u8; CMD_SIZE], fd: Option<BorrowedFd<'_>>) -> Sent {
        let mut w = self.0.borrow_mut();
        if w.never_reads {
            w.full_run += 1;
            w.fulls += 1;
            assert!(
                w.full_run <= 32,
                "the relay retried a full send in a loop ({} Full results in one entry): a send must never block",
                w.full_run
            );
            return Sent::Full;
        }
        let id = fd.map(|f| fd_inode(f).expect("a live descriptor"));
        let cmd = Cmd::decode(rec).expect("reserved1 is zero");
        w.sent.push((cmd, id));
        Sent::Done
    }
    fn recv(&mut self, _s: &Sock, buf: &mut [u8]) -> Recv {
        let mut w = self.0.borrow_mut();
        if std::mem::take(&mut w.fd_drop) {
            return Recv::FdDropped;
        }
        if w.inbox.is_empty() {
            return if w.eof { Recv::Closed } else { Recv::Empty };
        }
        let n = buf.len().min(w.inbox.len());
        for b in buf.iter_mut().take(n) {
            *b = w.inbox.pop_front().unwrap_or(0);
        }
        Recv::Bytes(n)
    }
}

#[derive(Default)]
struct H {
    timer: Option<u64>,
    /// The relay asked to be woken when the socket is writable.
    want_write: bool,
}

impl Host for H {
    fn watch(&mut self, _fd: i32, _r: bool, w: bool) {
        self.want_write = w;
    }
    fn timer(&mut self, d: Option<u64>) {
        self.timer = d;
    }
}

// ------------------------------------------------------------------------------------------------
// the fake broker: what it sends, and what the relay MAY send back

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// One tick per refresh (plus jitter).
    Steady,
    /// `mult` times the refresh rate; `burst > 1` sends that many back to back, then waits.
    Flood { mult: u32, burst: u32 },
    /// No ticks (the broker still reads and answers).
    Silent,
    /// Gaps cycling through `PATTERNS[i]` (milliseconds): the flapping broker.
    Pattern(usize),
}

/// Tick gaps around the 50 ms boundary, in milliseconds.
const PATTERNS: [&[u64]; 4] = [&[49, 51], &[55], &[40, 60], &[49, 50, 51, 52, 48]];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WantPolicy {
    /// A correct broker: a request only when the previous one was answered AND presented (or 100 ms).
    Lockstep,
    /// Every tick asks.
    Always,
    /// No tick asks.
    Never,
    /// A tick asks with this probability (permille).
    Random(u32),
}

#[derive(Clone, Debug)]
struct BCfg {
    period: u64,
    jitter: u64,
    want: WantPolicy,
    noisy: bool,
    advertise: bool,
    dup: u32,
    stale: u32,
    reorder: u32,
    drop: u32,
    backwards: u32,
    present: (u64, u64),
    release: (u64, u64),
}

impl BCfg {
    fn steady() -> BCfg {
        BCfg {
            period: P60,
            jitter: 0,
            want: WantPolicy::Lockstep,
            noisy: false,
            advertise: true,
            dup: 0,
            stale: 0,
            reorder: 0,
            drop: 0,
            backwards: 0,
            present: (MS, 8 * MS),
            release: (2 * MS, 20 * MS),
        }
    }
}

/// The next tick number after `s` (0 is skipped).
fn next_seq(s: u32) -> u32 {
    match s.wrapping_add(1) {
        0 => 1,
        n => n,
    }
}

#[derive(Clone, Copy)]
struct Meta {
    seq: u32,
    want: bool,
    fresh: bool,
}

struct Broker {
    cfg: BCfg,
    mode: Mode,
    /// The process is stopped (SIGSTOP / hung): no ticks, no noise, nothing read.
    hung: bool,
    rel_never: bool,
    /// The broker window is visible; false = occluded (no ticks at all).
    opted: bool,
    tick_seq: u32,
    pkt_seq: u32,
    next_tick_at: Option<u64>,
    last_emit_at: u64,
    pat_i: usize,
    next_noise_at: u64,
    held_back: Option<TickPkt>,
    // the broker's own lock-step view
    b_open: Option<(u32, u64)>,
    unpresented: Option<u64>,
    present_at: Option<u64>,
    attach_id: Option<u64>,
    shown: Option<u64>,
    releases: VecDeque<(u64, u64)>,
    // what the relay is EXPECTED to have seen (this connection)
    pushed: VecDeque<Meta>,
    cons_base: u64,
    exp_open: Option<u32>,
    last_fresh: Option<u32>,
    max_present: u64,
    last_label: Option<u32>,
    last_answer_ms: Option<u64>,
    e_ticks: u64,
    e_stale: u64,
    e_missed: u64,
    e_want: u64,
    e_back: u64,
    // statistics
    ticks_pushed: u64,
    noise_pushed: u64,
    stale_pushed: u64,
    commits: u64,
    answered: u64,
    unsolicited: u64,
    no_changes: u64,
    releases_pushed: u64,
    pattern_long_gaps: u64,
}

impl Broker {
    fn new(cfg: BCfg, mode: Mode) -> Broker {
        Broker {
            cfg,
            mode,
            hung: false,
            rel_never: false,
            opted: false,
            tick_seq: 0,
            pkt_seq: 0,
            next_tick_at: None,
            last_emit_at: 0,
            pat_i: 0,
            next_noise_at: 0,
            held_back: None,
            b_open: None,
            unpresented: None,
            present_at: None,
            attach_id: None,
            shown: None,
            releases: VecDeque::new(),
            pushed: VecDeque::new(),
            cons_base: 0,
            exp_open: None,
            last_fresh: None,
            max_present: 0,
            last_label: None,
            last_answer_ms: None,
            e_ticks: 0,
            e_stale: 0,
            e_missed: 0,
            e_want: 0,
            e_back: 0,
            ticks_pushed: 0,
            noise_pushed: 0,
            stale_pushed: 0,
            commits: 0,
            answered: 0,
            unsolicited: 0,
            no_changes: 0,
            releases_pushed: 0,
            pattern_long_gaps: 0,
        }
    }

    /// A new connection: a fresh broker instance (tick numbers start over, nothing is open).
    fn on_connect(&mut self, wire: &mut Wire, now: u64, cons_base: u64) {
        let keep = (self.cfg.clone(), self.mode, self.hung, self.rel_never);
        let stats = (
            self.ticks_pushed,
            self.noise_pushed,
            self.stale_pushed,
            self.commits,
            self.answered,
            self.unsolicited,
            self.no_changes,
            self.releases_pushed,
        );
        *self = Broker::new(keep.0, keep.1);
        self.hung = keep.2;
        self.rel_never = keep.3;
        (
            self.ticks_pushed,
            self.noise_pushed,
            self.stale_pushed,
            self.commits,
            self.answered,
            self.unsolicited,
            self.no_changes,
            self.releases_pushed,
        ) = stats;
        self.cons_base = cons_base;
        self.next_noise_at = now;
        let caps = CAP_DMABUF
            | CAP_FOCUS_EVENTS
            | CAP_RELEASE
            | if self.cfg.advertise { CAP_TICK } else { 0 };
        self.push_raw(
            wire,
            Pkt {
                ty: EV_HELLO,
                w0: 2,
                w1: caps,
                ..Pkt::default()
            },
        );
    }

    fn push_raw(&mut self, wire: &mut Wire, mut p: Pkt) -> bool {
        self.pkt_seq = self.pkt_seq.wrapping_add(1);
        p.seq = self.pkt_seq;
        wire.push(&p.encode())
    }

    /// Push one tick, recording what the relay must make of it.
    fn push_tick(&mut self, wire: &mut Wire, t: TickPkt) {
        self.pkt_seq = self.pkt_seq.wrapping_add(1);
        let p = Pkt::tick_pkt(self.pkt_seq, 0, &t);
        if !wire.push(&p.encode()) {
            return;
        }
        self.register_tick(&t);
    }

    /// Record what the relay must make of tick `t` once it reads it.
    fn register_tick(&mut self, t: &TickPkt) {
        self.ticks_pushed += 1;
        let fresh = self.last_fresh.is_none_or(|l| seq_newer(t.tick_seq, l));
        if fresh {
            if let Some(l) = self.last_fresh {
                let m = t.tick_seq.wrapping_sub(l).saturating_sub(1);
                self.e_missed += u64::from(if t.tick_seq < l { m.saturating_sub(1) } else { m });
            }
            self.last_fresh = Some(t.tick_seq);
            self.e_ticks += 1;
            self.e_want += u64::from(t.want);
            if t.present_ns < self.max_present {
                self.e_back += 1;
            }
            self.max_present = self.max_present.max(t.present_ns);
        } else {
            self.e_stale += 1;
            self.stale_pushed += 1;
        }
        self.pushed.push_back(Meta {
            seq: t.tick_seq,
            want: t.want,
            fresh,
        });
    }

    fn decide_want(&mut self, now: u64, rng: &mut Rng) -> bool {
        match self.cfg.want {
            WantPolicy::Always => true,
            WantPolicy::Never => false,
            WantPolicy::Random(p) => rng.chance(p),
            WantPolicy::Lockstep => {
                if self.unpresented.is_some_and(|d| now >= d + 100 * MS) {
                    self.unpresented = None;
                }
                let timed_out = self.b_open.is_some_and(|(_, at)| now >= at + 100 * MS);
                (self.b_open.is_none() || timed_out) && self.unpresented.is_none()
            }
        }
    }

    /// Emit one refresh's tick (with the configured chaos).
    fn emit_tick(&mut self, wire: &mut Wire, now: u64, rng: &mut Rng) {
        let want = self.decide_want(now, rng);
        self.tick_seq = next_seq(self.tick_seq);
        let present = if rng.chance(self.cfg.backwards) {
            now.saturating_sub(rng.range(20, 60) * MS)
        } else {
            now
        };
        let t = TickPkt {
            tick_seq: self.tick_seq,
            refresh_ns: u32::try_from(self.cfg.period).unwrap_or(0),
            want,
            clock_only: false,
            present_ns: present,
        };
        if want {
            self.b_open = Some((t.tick_seq, now));
        }
        if rng.chance(self.cfg.drop) {
            return;
        }
        if let Some(h) = self.held_back.take() {
            self.push_tick(wire, t);
            self.push_tick(wire, h); // out of order: the older one arrives last
        } else if rng.chance(self.cfg.reorder) {
            self.held_back = Some(t);
            return;
        } else {
            self.push_tick(wire, t);
        }
        if rng.chance(self.cfg.dup) {
            self.push_tick(wire, t);
        }
        if rng.chance(self.cfg.stale) {
            let mut s = self.tick_seq;
            for _ in 0..rng.range(1, 40) {
                s = s.wrapping_sub(1);
                if s == 0 {
                    s = u32::MAX;
                }
            }
            self.push_tick(
                wire,
                TickPkt {
                    tick_seq: s,
                    want: rng.chance(500),
                    ..t
                },
            );
        }
    }

    /// Interleaved traffic of every other kind: pacing, releases, pointer/cursor-ish state, unknown types.
    fn emit_noise(&mut self, wire: &mut Wire, rng: &mut Rng) -> u16 {
        let k = rng.below(8);
        let p = match k {
            0 => Pkt {
                ty: EV_FRAME,
                ..Pkt::default()
            },
            1 => Pkt {
                ty: EV_RELEASE,
                w0: rng.next() as u32,
                w1: 0x7000,
                ..Pkt::default()
            },
            2 => Pkt {
                ty: EV_POINTER,
                x: rng.below(2) as i32,
                ..Pkt::default()
            },
            3 => Pkt {
                ty: EV_FOCUS,
                x: 1,
                ..Pkt::default()
            },
            4 => Pkt {
                ty: EV_ABS,
                x: rng.below(1920) as i32,
                y: rng.below(1080) as i32,
                w0: 1920,
                w1: 1080,
                ..Pkt::default()
            },
            5 => Pkt {
                ty: EV_SURFACE,
                x: 1920,
                y: 1080,
                w0: 60_000,
                ..Pkt::default()
            },
            6 => Pkt {
                ty: EV_DEVICE,
                ..Pkt::default()
            },
            _ => Pkt {
                ty: 900 + rng.below(90) as u16,
                x: rng.next() as i32,
                ..Pkt::default()
            },
        };
        if self.push_raw(wire, p) {
            self.noise_pushed += 1;
        }
        p.ty
    }
}

// ------------------------------------------------------------------------------------------------
// scenarios

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GKind {
    Imm,
    Vsync,
    Alt,
    Rand,
}

/// A scripted act of the adversary (or of the environment), at an absolute time.
#[derive(Clone, Debug)]
enum Act {
    Mode(Mode),
    Want(WantPolicy),
    /// SIGSTOP / a hung broker: nothing produced, nothing read.
    Hung(bool),
    /// The broker stops reading (socket full) but goes on sending.
    NeverRead(bool),
    /// The broker process dies / starts again.
    Down,
    Up,
    NoRelease(bool),
    /// The first `n` bytes of a tick packet, then nothing at all.
    Partial(usize),
    /// A packet that violates the framing rules (kind `k`), or a descriptor on the stream (`k == 9`).
    Hostile(u8),
    /// After a stall: a burst of stale ticks (WANT set), stale releases.
    Resume(u32),
    /// The guest renders nothing for this long.
    GuestPause(u64),
    /// The broker crashes the moment the next ATTACH arrives (mid-frame).
    CrashOnAttach,
    /// The next tick comes exactly this long after the previous one.
    Gap(u64),
    /// The rest of a partial packet arrives.
    Complete,
}

#[derive(Clone, Debug)]
struct Sc {
    name: &'static str,
    gk: GKind,
    buffers: Option<usize>,
    render: (u64, u64),
    copy: (u64, u64),
    wake: u64,
    /// Chances (permille) per rendered frame of a burst (0-time renders) / a pause.
    burst_pm: u32,
    pause_pm: u32,
    relay: bool,
    broker_up: bool,
    b: BCfg,
    mode0: Mode,
    timeline: Vec<(u64, Act)>,
    /// Random mode/act changes every 20 ms..2 s (the adversarial scheduler).
    chaos: bool,
    flips: u64,
    max_ns: u64,
    /// The relay's counters must equal the broker's expectations at the end.
    exact: bool,
    /// The worker resets its gate at a new connection (what a fix of the defect §R1 would do).
    reset_gate: bool,
    /// Fixed-step time (the phase-1 loop), for the bit-identical headless run.
    fixed_dt: Option<u64>,
    /// The run lasts at least this long (a stall scenario must outlive its stall).
    min_sim_ns: u64,
    /// The relay's main loop is busy: it takes a socket turn at most this often.
    turn_gap_ns: u64,
    /// `on_socket` caps drawn from this set.
    caps: Vec<usize>,
    /// Expect every violation to be dropped (hostile framing).
    hostile: bool,
}

impl Sc {
    fn new(name: &'static str) -> Sc {
        Sc {
            name,
            gk: GKind::Imm,
            buffers: None,
            render: (300_000, 3 * MS),
            copy: (MS, 4 * MS),
            wake: MS / 2,
            burst_pm: 0,
            pause_pm: 0,
            relay: true,
            broker_up: true,
            b: BCfg::steady(),
            mode0: Mode::Steady,
            timeline: Vec::new(),
            chaos: false,
            flips: 2000,
            max_ns: 600_000 * MS,
            exact: false,
            reset_gate: true,
            fixed_dt: None,
            min_sim_ns: 0,
            turn_gap_ns: 0,
            caps: vec![READ_BATCH],
            hostile: false,
        }
    }
}

// ------------------------------------------------------------------------------------------------
// the world

#[derive(Default, Clone, Debug)]
struct Out {
    seed: u64,
    sim_ns: u64,
    submitted: u64,
    completed: u64,
    imm: u64,
    ticks: u64,
    free_ticks: u64,
    broker_ticks: u64,
    copies: u64,
    skipped: u64,
    latches: u64,
    handoffs: (u64, u64),
    max_tick_gap_ns: u64,
    min_broker_gap_ns: u64,
    frames_published: u64,
    commits: u64,
    answered: u64,
    unsolicited: u64,
    no_changes: u64,
    lost: u64,
    connects: u32,
    gate: crate::lockstep::GateCounters,
    relay: kf_broker::TickCounters,
    relay_c: kf_broker::conn::Counters,
    mailbox_coalesced: u64,
    sock_full_drops: u64,
    ticks_pushed: u64,
    stale_pushed: u64,
    noise_pushed: u64,
    fulls: u64,
    steps: u64,
    max_turn_packets: u64,
    max_held: u32,
    log_lines: usize,
    worst_flip_ns: u64,
    probation: u64,
    max_inbox: usize,
    last_flip_done_ns: u64,
    hostile: u64,
    goal: u64,
    last_commit_ns: u64,
    partial_drop_ms: u64,
    long_gaps: u64,
    relay_on: bool,
    advertised: bool,
    presents: u64,
    tearing: u64,
    base: Option<Base>,
}

#[derive(Clone, Copy, Debug)]
struct Base {
    completed: u64,
    copies: u64,
    ticks: u64,
    skipped: u64,
    presents: u64,
    tearing: u64,
}

struct Guest {
    kind: GKind,
    buffers: usize,
    k_render: u64,
    render_until: Option<u64>,
    ready: VecDeque<u64>,
    outstanding: Option<(u64, bool)>,
    completed: u64,
    wakes: VecDeque<u64>,
    burst_left: u32,
    pause_until: u64,
    submitted: u64,
    imm: u64,
}

enum Entry {
    Socket { cap: usize },
    Frame,
    Timer,
}

struct World {
    seed: u64,
    sc: Sc,
    rng: Rng,
    now: u64,
    trace: Trace,
    // display
    e: Engine,
    win_w: Writer,
    pacer: Pacer,
    gate: CreditGate,
    mdl: Model,
    tick_seen: u64,
    share: Arc<TickShare>,
    guest: Guest,
    fill_slot: Option<usize>,
    serial: u64,
    // relay side
    wire: Rc<RefCell<Wire>>,
    host: H,
    ring: Arc<FrameRing>,
    relay: Option<Relay<FakeLink>>,
    broker: Broker,
    seen_connects: u32,
    last_fresh_rx_ms: Option<u64>,
    was_connected: bool,
    crash_on_attach: bool,
    // bookkeeping
    script: VecDeque<(u64, Act)>,
    next_chaos_at: u64,
    out: Out,
    last_tick: Option<(u64, Source)>,
    last_vseq: u64,
    free_ticks: u64,
    polled: Option<u32>,
    distinct_polled: u64,
    defer_ready: Option<(u32, u64)>,
    stepped_same: u32,
    last_turn_ns: u64,
    base: Option<Base>,
    kick_write: bool,
    partial_rest: Option<(Vec<u8>, TickPkt)>,
    partial_at: Option<u64>,
    out_hostile: u64,
    t0: std::time::Instant,
}

const CALLS: usize = 192;
/// The bound on the vblank's silence: the broker's 50 ms, then up to two periods to the next free tick.
const TICK_GAP_BOUND: u64 = 50 * MS + 2 * P60 + MS;
/// The bound on a flip's life: a tick wait, a copy and a wake.
const FLIP_BOUND: u64 = TICK_GAP_BOUND + 40 * MS;
const INBOX_CAP: usize = 4096 * PKT_SIZE;

impl World {
    fn new(sc: Sc, seed: u64) -> World {
        let mut rng = Rng::new(seed);
        let mut e = engine();
        let _ = lit(&mut e);
        let mut pacer = Pacer::new(60);
        pacer.on_heads(
            &[HeadMode {
                head: 0,
                period_ns: P60,
                raster: (2200, 1125),
            }],
            0,
        );
        let ring = Arc::new(FrameRing::new(kf_broker::slots::BROKER_SLOTS, true));
        for j in 0..ring.slots() {
            let mem = SharedRam::create_named(c"kfb-soak-frame", 64 * 1024).unwrap();
            ring.install(j, SlotFds::new(mem, None).unwrap()).unwrap();
        }
        let wire = Rc::new(RefCell::new(Wire {
            up: sc.broker_up,
            inbox_cap: INBOX_CAP,
            ..Wire::default()
        }));
        let share = Arc::new(TickShare::new());
        share.set_cap_hz(60);
        let relay = sc.relay.then(|| {
            Relay::new(
                RelayConfig {
                    path: PathBuf::from("/run/soak/display.sock"),
                    extra_uid: None,
                },
                ring.clone(),
                FakeLink(wire.clone()),
            )
            .with_ticks(share.clone())
        });
        let buffers = sc.buffers.unwrap_or(2 + (seed % 2) as usize);
        let broker = Broker::new(sc.b.clone(), sc.mode0);
        let mut script: Vec<(u64, Act)> = sc.timeline.clone();
        script.sort_by_key(|(t, _)| *t);
        let guest = Guest {
            kind: sc.gk,
            buffers,
            k_render: 0,
            render_until: None,
            ready: VecDeque::new(),
            outstanding: None,
            completed: 0,
            wakes: VecDeque::new(),
            burst_left: 0,
            pause_until: 0,
            submitted: 0,
            imm: 0,
        };
        let next_chaos_at = rng.range(20 * MS, 200 * MS);
        World {
            seed,
            rng,
            now: 0,
            trace: Trace {
                ring: VecDeque::new(),
            },
            e,
            win_w: Writer::new(),
            pacer,
            gate: CreditGate::new(),
            mdl: Model::new(Worker::Mailbox),
            tick_seen: 0,
            share,
            guest,
            fill_slot: None,
            serial: 0,
            wire,
            host: H::default(),
            ring,
            relay,
            broker,
            seen_connects: 0,
            last_fresh_rx_ms: None,
            was_connected: false,
            crash_on_attach: false,
            script: script.into(),
            next_chaos_at,
            out: Out {
                seed,
                min_broker_gap_ns: u64::MAX,
                ..Out::default()
            },
            last_tick: None,
            last_vseq: 0,
            free_ticks: 0,
            polled: None,
            distinct_polled: 0,
            defer_ready: None,
            stepped_same: 0,
            last_turn_ns: 0,
            base: None,
            kick_write: false,
            partial_rest: None,
            partial_at: None,
            out_hostile: 0,
            t0: std::time::Instant::now(),
            sc,
        }
    }

    // ---- failure ------------------------------------------------------------------------------

    fn dump(&self) -> String {
        let mut s = String::new();
        for (t, e) in &self.trace.ring {
            s += &format!("  {:>12} ns  {e:?}\n", t);
        }
        s
    }

    fn fail(&self, msg: &str) -> ! {
        panic!(
            "\nlockstep_soak FAILED: scenario `{}` seed {} at t={} ns ({} ms): {msg}\n--- the last {} events ---\n{}replay: KF_SOAK_SEED={} cargo test --release -p kf-disp --lib {}\n",
            self.sc.name,
            self.seed,
            self.now,
            self.now / MS,
            self.trace.ring.len(),
            self.dump(),
            self.seed,
            self.sc.name
        );
    }

    fn tr(&mut self, e: Ev) {
        self.trace.push(self.now, e);
    }

    // ---- the relay ------------------------------------------------------------------------------

    fn relay_entry(&mut self, entry: Entry) {
        let Some(mut relay) = self.relay.take() else {
            return;
        };
        let now_ms = self.now / MS;
        self.wire.borrow_mut().full_run = 0;
        let pk0 = relay.counters().packets;
        let c0 = relay.tick_counters();
        let consumed0 = c0.ticks + c0.ticks_stale;
        let mut out: Vec<Input> = Vec::new();
        let (is_socket, cap) = match entry {
            Entry::Socket { cap } => {
                let w = self.wire.borrow();
                let readable = !w.inbox.is_empty() || w.eof;
                let writable = self.host.want_write && !w.never_reads;
                drop(w);
                relay.on_socket(now_ms, readable, writable, &mut self.host, &mut out, cap);
                (true, cap)
            }
            Entry::Frame => {
                relay.on_frame(now_ms, &mut self.host);
                (false, 0)
            }
            Entry::Timer => {
                relay.on_timer(now_ms, &mut self.host);
                (false, 0)
            }
        };
        let dpk = relay.counters().packets - pk0;
        self.out.max_turn_packets = self.out.max_turn_packets.max(dpk);
        if is_socket && dpk > cap.clamp(1, READ_BATCH) as u64 {
            self.relay = Some(relay);
            self.fail(&format!(
                "a relay turn processed {dpk} packets, the per-turn budget is {}",
                cap.clamp(1, READ_BATCH)
            ));
        }
        if !is_socket && dpk != 0 {
            self.relay = Some(relay);
            self.fail("a non-socket entry read packets");
        }
        if out.len() > cap.max(1).min(READ_BATCH) {
            self.relay = Some(relay);
            self.fail("a turn delivered more inputs than its cap");
        }
        self.after_entry(&mut relay, consumed0);
        self.relay = Some(relay);
    }

    fn after_entry(&mut self, relay: &mut Relay<FakeLink>, consumed0: u64) {
        let now_ms = self.now / MS;
        // a new connection: a fresh broker instance
        let connects = self.wire.borrow().connects;
        if connects != self.seen_connects {
            self.seen_connects = connects;
            let c = relay.tick_counters();
            let base = c.ticks + c.ticks_stale;
            self.broker
                .on_connect(&mut self.wire.borrow_mut(), self.now, base);
            self.last_fresh_rx_ms = None;
            if self.sc.reset_gate {
                self.gate.reset();
            }
            self.tr(Ev::Connect);
        }
        let connected = relay.connected();
        if self.was_connected && !connected {
            self.out.lost += 1;
            self.tr(Ev::Lost);
            if let Some(at) = self.partial_at.take() {
                self.out.partial_drop_ms = (self.now - at) / MS;
                // the stalled broker is restarted (the relay dropped it at the read deadline)
                self.broker.hung = false;
                self.partial_rest = None;
                self.wire.borrow_mut().never_reads = false;
            }
        }
        self.was_connected = connected;
        // the ticks this entry consumed
        let c = relay.tick_counters();
        let consumed = c.ticks + c.ticks_stale;
        let mut in_entry: Vec<Meta> = Vec::new();
        for _ in consumed0..consumed {
            match self.broker.pushed.pop_front() {
                Some(m) => in_entry.push(m),
                None => {
                    let m = format!(
                        "the relay consumed a tick packet nobody pushed (consumed {consumed0}..{consumed})"
                    );
                    self.fail(&m)
                }
            }
        }
        let fresh: Vec<Meta> = in_entry.iter().filter(|m| m.fresh).copied().collect();
        // live before this entry (the silence rule at this entry's time, old last tick): a frame
        // without a request is wrong only if the ticks were live before AND after — the liveness can
        // change inside one batch of packets
        let live_before = tick_live(self.last_fresh_rx_ms, now_ms);
        if !fresh.is_empty() {
            self.last_fresh_rx_ms = Some(now_ms);
        }
        let wants: Vec<u32> = fresh.iter().filter(|m| m.want).map(|m| m.seq).collect();
        let exp_before = self.broker.exp_open;
        // every command the relay sent
        let cmds = std::mem::take(&mut self.wire.borrow_mut().sent);
        let negotiated = relay.ticks_negotiated();
        let live = relay.tick_live();
        let mut answered: Vec<u32> = Vec::new();
        for (cmd, id) in cmds {
            match cmd.ty {
                CMD_ATTACH => {
                    self.broker.attach_id = id;
                    if self.crash_on_attach {
                        self.crash_on_attach = false;
                        {
                            let mut w = self.wire.borrow_mut();
                            w.up = false;
                            w.eof = true;
                        }
                        self.tr(Ev::Act("crash on attach"));
                    }
                }
                CMD_WINDOW => {}
                CMD_CAPS => {
                    self.tr(Ev::Caps);
                    let advertised = self.broker.cfg.advertise;
                    if advertised {
                        if cmd.width != CLIENT_TICK || cmd.height != 60 {
                            self.fail(&format!("CAPS to a tick broker was {cmd:?}"));
                        }
                        self.broker.opted = true;
                        self.broker.next_tick_at = Some(self.now);
                    } else if cmd.width != 0 || cmd.height != 0 {
                        self.fail(&format!("CAPS opted in to a broker without CAP_TICK: {cmd:?}"));
                    }
                }
                CMD_COMMIT => {
                    self.broker.commits += 1;
                    self.out.last_commit_ns = self.now;
                    self.tr(Ev::Commit { seq: cmd.seq });
                    let label = cmd.seq;
                    if !self.broker.opted {
                        if label != 0 {
                            self.fail(&format!("COMMIT.seq {label} on a connection that never opted in"));
                        }
                    } else if label == 0 {
                        if live && live_before {
                            self.fail("a frame was sent WITHOUT a credit while the broker's ticks are live (COMMIT.seq 0)");
                        }
                        self.broker.unsolicited += 1;
                    } else {
                        let mut allowed = wants.clone();
                        allowed.extend(exp_before);
                        if !allowed.contains(&label) {
                            self.fail(&format!(
                                "COMMIT.seq {label} is not an open request (open before: {exp_before:?}, opened in this turn: {wants:?}) — a frame without a credit"
                            ));
                        }
                        if self.broker.last_label.is_some_and(|l| !seq_newer(label, l)) {
                            let l = self.broker.last_label;
                            self.fail(&format!("request {label} answered again (last answered {l:?}): more than one frame per credit"));
                        }
                        self.broker.last_label = Some(label);
                        self.broker.answered += 1;
                        answered.push(label);
                        if self.broker.b_open.map(|(s, _)| s) == Some(label) {
                            self.broker.b_open = None;
                        }
                        self.broker.last_answer_ms = Some(now_ms);
                    }
                    // the compositor takes it: the previous frame is released when this one is shown
                    let prev = self.broker.shown.take();
                    self.broker.shown = self.broker.attach_id;
                    let d = self.broker.cfg.present;
                    let at = self.now + self.rng.range(d.0, d.1);
                    self.broker.unpresented = Some(at);
                    self.broker.present_at = Some(at);
                    if let Some(p) = prev
                        && !self.broker.rel_never
                    {
                        let r = self.broker.cfg.release;
                        let rat = at + self.rng.range(r.0, r.1);
                        self.broker.releases.push_back((rat, p));
                    }
                }
                CMD_NO_CHANGE => {
                    self.tr(Ev::NoChange { seq: cmd.width });
                    let label = cmd.width;
                    if !self.broker.opted {
                        self.fail("NO_CHANGE on a connection that never opted in");
                    }
                    let mut allowed = wants.clone();
                    allowed.extend(exp_before);
                    if !allowed.contains(&label) {
                        self.fail(&format!(
                            "NO_CHANGE({label}) answers no open request (open before: {exp_before:?}, opened: {wants:?})"
                        ));
                    }
                    if fresh.len() == 1 && wants.first() == Some(&label) && exp_before != Some(label) {
                        self.fail(&format!("NO_CHANGE({label}) at the very tick that opened it"));
                    }
                    let gap = (self.broker.cfg.period / 2_000_000).max(1);
                    if let Some(t) = self.broker.last_answer_ms
                        && now_ms < t + gap
                    {
                        self.fail(&format!(
                            "NO_CHANGE at {now_ms} ms, only {} ms after the last answer (the pacing is {gap} ms)",
                            now_ms - t
                        ));
                    }
                    if self.broker.last_label.is_some_and(|l| !seq_newer(label, l)) {
                        self.fail(&format!("request {label} answered twice (NO_CHANGE)"));
                    }
                    self.broker.last_label = Some(label);
                    self.broker.no_changes += 1;
                    answered.push(label);
                    if self.broker.b_open.map(|(s, _)| s) == Some(label) {
                        self.broker.b_open = None;
                    }
                    self.broker.last_answer_ms = Some(now_ms);
                }
                other => self.fail(&format!("the relay sent an unexpected command type {other}")),
            }
        }
        self.broker.exp_open = match wants.last() {
            Some(w) => (!answered.contains(w)).then_some(*w),
            None => exp_before.filter(|o| !answered.contains(o)),
        };
        // the 50 ms rule: the relay is live exactly when a fresh tick arrived within 50 ms
        if negotiated {
            let expect = tick_live(self.last_fresh_rx_ms, now_ms);
            if expect != live {
                self.fail(&format!(
                    "the relay says ticks are live={live}, the 50 ms rule says {expect} (last fresh tick at {:?} ms, now {now_ms} ms)",
                    self.last_fresh_rx_ms
                ));
            }
        }
        let held = self.ring.held_mask().count_ones();
        self.out.max_held = self.out.max_held.max(held);
        if held > 2 {
            self.fail(&format!(
                "{held} frames in flight (held mask {:#x}): the bound is 2",
                self.ring.held_mask()
            ));
        }
    }
}

impl World {
    fn connected(&self) -> bool {
        self.relay.as_ref().is_some_and(Relay::connected) && {
            let w = self.wire.borrow();
            w.up && !w.eof
        }
    }

    // ---- the script and the adversary ------------------------------------------------------------

    fn schedule(&mut self, at: u64, a: Act) {
        let i = self.script.partition_point(|(t, _)| *t <= at);
        self.script.insert(i, (at, a));
    }

    fn step_script(&mut self) {
        while self.script.front().is_some_and(|(t, _)| *t <= self.now) {
            let Some((_, a)) = self.script.pop_front() else {
                break;
            };
            self.apply(a);
        }
        if self.sc.chaos && self.now >= self.next_chaos_at {
            self.chaos();
        }
    }

    fn apply(&mut self, a: Act) {
        let now = self.now;
        let opted_now = self.broker.opted;
        match a {
            Act::Mode(m) => {
                self.tr(Ev::Act("mode"));
                self.broker.mode = m;
                self.broker.next_tick_at = (opted_now && m != Mode::Silent).then_some(now);
            }
            Act::Want(p) => self.broker.cfg.want = p,
            Act::Hung(h) => {
                self.tr(Ev::Act(if h { "SIGSTOP" } else { "SIGCONT" }));
                self.broker.hung = h;
                self.wire.borrow_mut().never_reads = h;
                if !h {
                    self.broker.next_tick_at = (opted_now && self.broker.mode != Mode::Silent)
                        .then_some(now + MS);
                    self.kick_write = true;
                }
            }
            Act::NeverRead(b) => {
                self.tr(Ev::Act("never-read"));
                self.wire.borrow_mut().never_reads = b;
                self.kick_write = !b;
            }
            Act::Down => {
                self.tr(Ev::Act("broker down"));
                let mut w = self.wire.borrow_mut();
                w.up = false;
                w.eof = true;
            }
            Act::Up => {
                self.tr(Ev::Act("broker up"));
                self.wire.borrow_mut().up = true;
            }
            Act::NoRelease(b) => self.broker.rel_never = b,
            Act::Partial(n) => {
                self.tr(Ev::Act("partial packet"));
                let t = TickPkt {
                    tick_seq: next_seq(self.broker.tick_seq),
                    refresh_ns: P60 as u32,
                    want: true,
                    clock_only: false,
                    present_ns: now,
                };
                let bytes = Pkt::tick_pkt(0, 0, &t).encode();
                let n = n.clamp(1, PKT_SIZE - 1);
                self.wire.borrow_mut().push(&bytes[..n]);
                self.partial_rest = Some((bytes[n..].to_vec(), t));
                self.broker.hung = true;
                self.partial_at = Some(now);
            }
            Act::Hostile(k) => {
                if !self.connected() {
                    return;
                }
                self.tr(Ev::Act("hostile"));
                self.out_hostile += 1;
                let mut t = TickPkt {
                    tick_seq: next_seq(self.broker.tick_seq),
                    refresh_ns: P60 as u32,
                    want: true,
                    clock_only: false,
                    present_ns: now,
                };
                let mut p = match k {
                    0 => {
                        t.tick_seq = 0;
                        Pkt::tick_pkt(1, 0, &t)
                    }
                    1 => {
                        let mut p = Pkt::tick_pkt(1, 0, &t);
                        p.y = (p.y.cast_unsigned() | (1 << 30)).cast_signed();
                        p
                    }
                    2 => {
                        t.refresh_ns = 500_000;
                        Pkt::tick_pkt(1, 0, &t)
                    }
                    3 => {
                        let mut p = Pkt::tick_pkt(1, 0, &t);
                        p.y = (p.y.cast_unsigned() | (1 << 31)).cast_signed();
                        p
                    }
                    // a perfectly valid tick: a violation only on a connection that never opted in
                    5 => Pkt::tick_pkt(1, 0, &t),
                    _ => Pkt {
                        ty: EV_TICK,
                        ..Pkt::default()
                    },
                };
                if k == 9 {
                    self.wire.borrow_mut().fd_drop = true;
                    p = Pkt {
                        ty: EV_FRAME,
                        ..Pkt::default()
                    };
                }
                self.wire.borrow_mut().push(&p.encode());
            }
            Act::Resume(n) => {
                self.tr(Ev::Act("resume burst"));
                self.broker.hung = false;
                self.wire.borrow_mut().never_reads = false;
                self.kick_write = true;
                if self.connected() {
                    let base = self.broker.tick_seq;
                    for i in 0..n {
                        let mut s = base;
                        for _ in 0..=i {
                            s = s.wrapping_sub(1);
                            if s == 0 {
                                s = u32::MAX;
                            }
                        }
                        let t = TickPkt {
                            tick_seq: s,
                            refresh_ns: P60 as u32,
                            want: true,
                            clock_only: false,
                            present_ns: now.saturating_sub(100 * MS),
                        };
                        {
                            let mut w = self.wire.borrow_mut();
                            self.broker.push_tick(&mut w, t);
                        }
                        self.tr(Ev::Stale { seq: s });
                    }
                    for _ in 0..(n / 4) {
                        let mut w = self.wire.borrow_mut();
                        self.broker.push_raw(
                            &mut w,
                            Pkt {
                                ty: EV_RELEASE,
                                w0: self.rng.next() as u32,
                                w1: 0x7000,
                                ..Pkt::default()
                            },
                        );
                    }
                    self.broker.next_tick_at = (opted_now && self.broker.mode != Mode::Silent)
                        .then_some(now + MS);
                }
            }
            Act::GuestPause(d) => self.guest.pause_until = now + d,
            Act::Gap(g) => {
                self.tr(Ev::Act("gap"));
                self.broker.next_tick_at =
                    Some((self.broker.last_emit_at + g).max(now)).filter(|_| opted_now);
            }
            Act::Complete => {
                if let Some((rest, t)) = self.partial_rest.take() {
                    self.tr(Ev::Act("partial completed"));
                    self.wire.borrow_mut().push(&rest);
                    self.broker.register_tick(&t);
                    self.broker.hung = false;
                    self.partial_at = None;
                }
            }
            Act::CrashOnAttach => self.crash_on_attach = true,
        }
    }

    /// The adversarial scheduler: a random change every 20 ms..2 s, with its own undo.
    fn chaos(&mut self) {
        let now = self.now;
        let mut rng = self.rng.clone();
        let r = &mut rng;
        let k = r.below(100);
        let dur = |r: &mut Rng, lo: u64, hi: u64| r.range(lo, hi);
        let mut next = r.range(20 * MS, 600 * MS);
        match k {
            0..=29 => {
                let p = match r.below(10) {
                    0..=6 => WantPolicy::Lockstep,
                    7 => WantPolicy::Always,
                    8 => WantPolicy::Random(300),
                    _ => WantPolicy::Never,
                };
                self.apply(Act::Want(p));
                self.apply(Act::Mode(Mode::Steady));
            }
            30..=44 => {
                let mult = [10, 100, 1000][r.below(3) as usize];
                let burst = [1, 1, 8, 64][r.below(4) as usize];
                let d = dur(r, 10 * MS, 200 * MS);
                self.apply(Act::Mode(Mode::Flood { mult, burst }));
                self.schedule(now + d, Act::Mode(Mode::Steady));
            }
            45..=59 => {
                let d = [10, 49, 50, 51, 55, 200, 2000][r.below(7) as usize] * MS;
                self.apply(Act::Mode(Mode::Silent));
                self.schedule(now + d, Act::Resume(r.range(0, 30) as u32));
                self.schedule(now + d, Act::Mode(Mode::Steady));
                next = d + r.range(5 * MS, 100 * MS);
            }
            60..=64 => {
                let d = dur(r, 5 * MS, 300 * MS);
                self.apply(Act::Hung(true));
                self.schedule(now + d, Act::Resume(r.range(0, 30) as u32));
                next = d + r.range(5 * MS, 100 * MS);
            }
            65..=69 => {
                let d = dur(r, 20 * MS, 300 * MS);
                self.apply(Act::NeverRead(true));
                self.schedule(now + d, Act::NeverRead(false));
                next = d + r.range(5 * MS, 100 * MS);
            }
            70..=74 => {
                let d = dur(r, 10 * MS, 800 * MS);
                self.apply(Act::Down);
                self.schedule(now + d, Act::Up);
                next = d + r.range(300 * MS, 900 * MS);
            }
            75..=79 => {
                let d = dur(r, 50 * MS, 1500 * MS);
                self.apply(Act::NoRelease(true));
                self.schedule(now + d, Act::NoRelease(false));
            }
            80..=81 => {
                let n = r.range(1, 23) as usize;
                self.apply(Act::Partial(n));
                // the relay drops the connection at the read deadline; the broker comes back after
                self.schedule(now + PARTIAL_READ_MS * MS + 100 * MS, Act::Hung(false));
                next = PARTIAL_READ_MS * MS + 400 * MS;
            }
            82..=86 => {
                let d = dur(r, 50 * MS, 300 * MS);
                self.apply(Act::GuestPause(d));
            }
            87..=88 => {
                self.apply(Act::CrashOnAttach);
                self.schedule(now + 400 * MS, Act::Up);
                next = 700 * MS;
            }
            89..=91 => {
                let kind = [0u8, 1, 2, 3, 9][r.below(5) as usize];
                self.apply(Act::Hostile(kind));
                next = 400 * MS;
            }
            _ => {
                self.apply(Act::Mode(Mode::Steady));
            }
        }
        self.rng = rng;
        self.next_chaos_at = now + next;
    }

    fn step_broker(&mut self) {
        if self.relay.is_none() || self.broker.hung || !self.connected() {
            return;
        }
        let now = self.now;
        if self.broker.present_at.is_some_and(|t| now >= t) {
            self.broker.unpresented = None;
            self.broker.present_at = None;
        }
        while let Some(&(at, id)) = self.broker.releases.front() {
            if at > now {
                break;
            }
            self.broker.releases.pop_front();
            {
                let mut w = self.wire.borrow_mut();
                self.broker.push_raw(
                    &mut w,
                    Pkt {
                        ty: EV_RELEASE,
                        w0: id as u32,
                        w1: (id >> 32) as u32,
                        ..Pkt::default()
                    },
                );
            }
            self.broker.releases_pushed += 1;
            self.tr(Ev::Release);
        }
        if self.broker.cfg.noisy && now >= self.broker.next_noise_at {
            let mut w = self.wire.borrow_mut();
            let ty = self.broker.emit_noise(&mut w, &mut self.rng);
            drop(w);
            self.tr(Ev::Noise(ty));
            self.broker.next_noise_at = now + self.rng.range(100_000, 3 * MS);
        }
        if !self.broker.opted || self.broker.mode == Mode::Silent {
            return;
        }
        let mut spins = 0;
        while self.broker.next_tick_at.is_some_and(|t| t <= now) {
            spins += 1;
            assert!(spins < 100_000, "the broker model spun");
            let mut w = self.wire.borrow_mut();
            let (period, jitter) = (self.broker.cfg.period, self.broker.cfg.jitter);
            match self.broker.mode {
                Mode::Steady => {
                    self.broker.emit_tick(&mut w, now, &mut self.rng);
                    drop(w);
                    self.tr(Ev::Tick {
                        seq: self.broker.tick_seq,
                        want: self.broker.b_open.is_some_and(|(s, _)| s == self.broker.tick_seq),
                    });
                    let j = self.rng.below(jitter + 1);
                    self.broker.next_tick_at = Some(now + period + j);
                }
                Mode::Flood { mult, burst } => {
                    let iv = (period / u64::from(mult.max(1))).max(1);
                    let b = burst.max(1);
                    for _ in 0..b {
                        self.broker.emit_tick(&mut w, now, &mut self.rng);
                    }
                    drop(w);
                    self.tr(Ev::Tick {
                        seq: self.broker.tick_seq,
                        want: self.broker.b_open.is_some_and(|(s, _)| s == self.broker.tick_seq),
                    });
                    self.broker.next_tick_at = Some(now + u64::from(b) * iv);
                }
                Mode::Pattern(pi) => {
                    self.broker.emit_tick(&mut w, now, &mut self.rng);
                    drop(w);
                    let pat = PATTERNS[pi % PATTERNS.len()];
                    let g = pat[self.broker.pat_i % pat.len()];
                    self.broker.pat_i += 1;
                    self.broker.pattern_long_gaps += u64::from(g > 50);
                    self.broker.next_tick_at = Some(now + g * MS);
                }
                Mode::Silent => {
                    self.broker.next_tick_at = None;
                }
            }
            self.broker.last_emit_at = now;
        }
    }

    fn step_relay(&mut self) {
        if self.relay.is_none() {
            return;
        }
        if self.host.timer.is_some_and(|t| t.saturating_mul(MS) <= self.now) {
            self.relay_entry(Entry::Timer);
        }
        let go = {
            let w = self.wire.borrow();
            let conn = self.relay.as_ref().is_some_and(Relay::connected);
            conn && (!w.inbox.is_empty() || w.eof || self.kick_write)
        };
        if go && self.now >= self.last_turn_ns + self.sc.turn_gap_ns {
            self.last_turn_ns = self.now;
            self.kick_write = false;
            let cap = self.sc.caps[self.rng.below(self.sc.caps.len() as u64) as usize];
            self.relay_entry(Entry::Socket { cap });
        }
    }
}

impl World {
    // ---- the worker (a model of kf-qemu display.rs steps 4 and 6) ------------------------------------

    fn on_vblank(&mut self, t: &crate::pace::Tick) {
        let now = self.now;
        if t.seq != self.last_vseq + 1 {
            self.fail(&format!(
                "the vblank counter went {} -> {}: it must advance by exactly one",
                self.last_vseq, t.seq
            ));
        }
        self.last_vseq = t.seq;
        self.tr(Ev::Vblank {
            from: t.from,
            seq: t.seq,
        });
        if let Some((l, _)) = self.last_tick {
            let gap = now - l;
            self.out.max_tick_gap_ns = self.out.max_tick_gap_ns.max(gap);
            if t.from == Source::Broker {
                let guard = self.pacer.source(0).map_or(P60 / 2, |s| s.guard_ns());
                self.out.min_broker_gap_ns = self.out.min_broker_gap_ns.min(gap);
                if gap < guard {
                    self.fail(&format!(
                        "a DOUBLE TICK: a broker tick {gap} ns after the previous tick (the guard is {guard} ns)"
                    ));
                }
            }
        }
        self.last_tick = Some((now, t.from));
    }

    fn begin_copy_slot(&mut self) {
        self.tr(Ev::CopyStart);
        if self.relay.is_none() {
            return;
        }
        let Some(t) = self.ring.fill_target(None) else {
            self.fail("no free frame slot: the ring's cap argument broke");
        };
        let held = self.ring.held_mask();
        if held & (1 << t) != 0 {
            self.fail(&format!(
                "the fill target {t} is a HELD slot (held mask {held:#x}): the broker still reads it"
            ));
        }
        if self.ring.broker_ready() == Some(t) {
            self.fail("the fill target is the frame ready for the broker");
        }
        self.fill_slot = Some(t);
    }

    fn publish_frame(&mut self) {
        self.tr(Ev::CopyDone);
        if self.relay.is_none() {
            return;
        }
        let Some(slot) = self.fill_slot.take() else {
            return;
        };
        if !self.ring.is_free(slot) {
            self.fail(&format!("slot {slot} was taken while the copy filled it"));
        }
        self.serial += 1;
        self.ring.describe(
            slot,
            FrameGeom {
                width: 64,
                height: 64,
                stride: 256,
                fourcc: FOURCC_XR24,
                serial: self.serial,
                cursor: false,
            },
        );
        self.ring.publish(slot);
        let _ = self.ring.take_console();
        self.out.frames_published += 1;
        self.tr(Ev::Publish { slot });
        self.relay_entry(Entry::Frame);
    }

    fn worker_pass(&mut self) {
        let now = self.now;
        // 4. vblanks whose time has come: the free clock's, and the broker's through the share
        let mut got = self.pacer.due(now);
        if self.relay.is_some()
            && let Some(snap) = self.share.snapshot(self.tick_seen)
        {
            let coalesced = snap.count.saturating_sub(self.tick_seen + 1);
            self.tick_seen = snap.count;
            let want = self.share.take_want();
            let t = if self.gate.fresh(snap.tick_seq) {
                self.pacer.broker_tick(0, now)
            } else {
                None
            };
            let (last, guard) = self
                .pacer
                .source(0)
                .map_or((None, 0), |s| (s.last_tick_ns(), s.guard_ns()));
            self.gate.on_tick(
                snap.tick_seq,
                coalesced,
                want,
                t.map(|_| now),
                last,
                guard,
            );
            if let Some(req) = want {
                self.defer_ready = t
                    .is_none()
                    .then(|| (req, last.map_or(0, |l| l.saturating_add(guard))));
            }
            got.extend(t);
        }
        if let Some(req) = self.gate.poll(now) {
            self.tr(Ev::Grant { req });
            if let Some((r, at)) = self.defer_ready
                && r == req
                && now < at
            {
                self.fail(&format!(
                    "credit {req} was acted on at {now} ns, before the double-tick guard's edge at {at} ns"
                ));
            }
            if self.polled != Some(req) {
                self.polled = Some(req);
                self.distinct_polled += 1;
            }
            self.mdl.mailbox.grant();
        }
        for (_, ho, _) in self.pacer.take_handoffs() {
            self.tr(Ev::Handoff(ho));
        }
        for t in got {
            self.on_vblank(&t);
            let s = self.e.vblank(0, &mut |_a: &Acquire| true);
            self.mdl.effects(&s);
            if t.from == Source::Free {
                self.mdl.tick();
                self.free_ticks += 1;
                self.out.free_ticks += 1;
            } else {
                self.out.broker_ticks += 1;
            }
            self.out.ticks += 1;
        }
        // 6. the copy completes, and what was waiting for it goes out
        let (done0, started0) = (self.mdl.done, self.mdl.started);
        let copy = self.rng.range(self.sc.copy.0, self.sc.copy.1);
        let published = self.mdl.complete(now, copy);
        if self.mdl.done > done0 {
            self.publish_frame();
        }
        if self.mdl.started > started0 {
            self.begin_copy_slot();
        }
        for _ in 0..published {
            self.guest.wakes.push_back(now + self.sc.wake);
        }
        self.guest_pass();
        let (done1, started1) = (self.mdl.done, self.mdl.started);
        let copy = self.rng.range(self.sc.copy.0, self.sc.copy.1);
        for _ in 0..self.mdl.publish() {
            self.guest.wakes.push_back(now + self.sc.wake);
        }
        self.mdl.pass(now, copy);
        debug_assert_eq!(done1, self.mdl.done);
        if self.mdl.started > started1 {
            self.begin_copy_slot();
        }
    }

    // ---- the guest -----------------------------------------------------------------------------------

    fn render_ns(&mut self) -> u64 {
        let g = &mut self.guest;
        if g.burst_left > 0 {
            g.burst_left -= 1;
            return 0;
        }
        if self.rng.chance(self.sc.burst_pm) {
            self.guest.burst_left = self.rng.range(5, 40) as u32;
            return 0;
        }
        if self.rng.chance(self.sc.pause_pm) {
            self.guest.pause_until = self.now + self.rng.range(100 * MS, 300 * MS);
        }
        self.rng.range(self.sc.render.0, self.sc.render.1)
    }

    fn guest_pass(&mut self) {
        let now = self.now;
        // 3. the guest learns of a completion
        while self.guest.wakes.front().is_some_and(|t| *t <= now) {
            self.guest.wakes.pop_front();
            if let Some((at, _)) = self.guest.outstanding.take() {
                let age = now - at;
                self.out.worst_flip_ns = self.out.worst_flip_ns.max(age);
                self.out.last_flip_done_ns = now;
            }
            self.guest.completed += 1;
            self.tr(Ev::Done);
        }
        // 4. render
        if let Some(t) = self.guest.render_until
            && now >= t
        {
            self.guest.render_until = None;
            self.guest.ready.push_back(self.guest.k_render);
            self.guest.k_render += 1;
        }
        let g = &self.guest;
        let free = g.k_render < g.buffers as u64 || g.completed + g.buffers as u64 >= g.k_render + 2;
        if self.guest.render_until.is_none() && free && now >= self.guest.pause_until {
            let r = self.render_ns();
            if now >= self.guest.pause_until {
                self.guest.render_until = Some(now + r);
            }
        }
        // 5. a flip: one outstanding at a time (the driver programs flip N after N-1 finished)
        if self.guest.outstanding.is_none()
            && let Some(k) = self.guest.ready.pop_front()
        {
            let imm = match self.guest.kind {
                GKind::Imm => true,
                GKind::Vsync => false,
                GKind::Alt => k % 2 == 0,
                GKind::Rand => self.rng.chance(500),
            };
            let methods = [
                (ma(WIN, "SET_CONTEXT_DMA_ISO", 0), 0x1_0001),
                (
                    m(WIN, "SET_PRESENT_CONTROL"),
                    put(0, fl(WIN, "SET_PRESENT_CONTROL_BEGIN_MODE"), u32::from(imm)),
                ),
                (m(WIN, "SET_WINDOW_INTERLOCK_FLAGS"), 0),
                (m(WIN, "UPDATE"), 0),
            ];
            let (bytes, p) = self.win_w.push(&methods);
            self.guest.outstanding = Some((now, imm));
            self.guest.submitted += 1;
            self.guest.imm += u64::from(imm);
            self.tr(Ev::Flip { imm });
            let s = self.e.step(1, &bytes, p, &mut |_a: &Acquire| true);
            if imm && !s.effects.iter().any(|e| matches!(e, Effect::Latched { .. })) {
                self.fail("an IMMEDIATE flip did not latch at once: it was parked");
            }
            self.mdl.effects(&s);
        }
    }

    // ---- the loop ------------------------------------------------------------------------------------

    fn next_event(&self) -> Option<u64> {
        if let Some(dt) = self.sc.fixed_dt {
            return Some(if self.out.steps == 0 { 0 } else { self.now + dt });
        }
        let mut t = u64::MAX;
        let mut at = |x: Option<u64>| {
            if let Some(x) = x {
                t = t.min(x.max(self.now));
            }
        };
        at(self.pacer.next_ns());
        at(self.gate.next_ns());
        at(self.mdl.inflight.map(|f| f.1));
        at(self.guest.wakes.front().copied());
        at(self.guest.render_until);
        let g = &self.guest;
        if g.outstanding.is_none() && !g.ready.is_empty() {
            at(Some(self.now));
        }
        if g.render_until.is_none() && self.now < g.pause_until {
            at(Some(g.pause_until));
        }
        if self.relay.is_some() {
            at(self.host.timer.map(|d| d.saturating_mul(MS)));
            let w = self.wire.borrow();
            let conn = self.relay.as_ref().is_some_and(Relay::connected);
            if conn && (!w.inbox.is_empty() || w.eof || self.kick_write) {
                at(Some(self.last_turn_ns + self.sc.turn_gap_ns));
            }
            drop(w);
            if !self.broker.hung && self.connected() {
                if self.broker.opted && self.broker.mode != Mode::Silent {
                    at(self.broker.next_tick_at);
                }
                at(self.broker.releases.front().map(|r| r.0));
                at(self.broker.present_at);
                if self.broker.cfg.noisy {
                    at(Some(self.broker.next_noise_at));
                }
            }
        }
        at(self.script.front().map(|s| s.0));
        if self.sc.chaos {
            at(Some(self.next_chaos_at));
        }
        (t != u64::MAX).then_some(t)
    }

    fn check(&mut self) {
        let now = self.now;
        if let Some((l, _)) = self.last_tick
            && now - l > TICK_GAP_BOUND
        {
            self.fail(&format!(
                "the vblank STALLED for {} ms (bound {} ms: the 50 ms silence plus two periods)",
                (now - l) / MS,
                TICK_GAP_BOUND / MS
            ));
        }
        if let Some((at, imm)) = self.guest.outstanding {
            let age = now - at;
            let bound = if imm {
                self.sc.copy.1 + self.sc.wake + 2 * MS
            } else {
                FLIP_BOUND
            };
            // an immediate flip may sit behind a vsync flip's items only when the guest is single-flip
            let bound = if self.sc.gk == GKind::Imm { bound } else { FLIP_BOUND };
            if age > bound {
                self.fail(&format!(
                    "a {} flip has not completed after {} ms (bound {} ms)",
                    if imm { "immediate" } else { "vsync" },
                    age / MS,
                    bound / MS
                ));
            }
        }
        if self.mdl.violations != 0 {
            self.fail(&format!(
                "the BUFFER-REUSE rule broke {} time(s): a flip completed under a copy of the buffer it replaced",
                self.mdl.violations
            ));
        }
        let w = self.wire.borrow();
        self.out.max_inbox = self.out.max_inbox.max(w.inbox.len());
        if w.inbox.len() > INBOX_CAP {
            drop(w);
            self.fail("the socket buffer grew past its bound");
        }
        drop(w);
        if self.mdl.copies > self.free_ticks + self.distinct_polled {
            let (c, f, d) = (self.mdl.copies, self.free_ticks, self.distinct_polled);
            self.fail(&format!(
                "{c} copies but only {f} free-clock ticks and {d} distinct broker credits: a copy ran without a credit"
            ));
        }
        if self.gate.pending().is_some() && self.gate.c.granted > self.gate.c.granted.max(0) {
            self.fail("unreachable");
        }
    }

    fn step(&mut self) {
        self.step_script();
        self.step_broker();
        self.step_relay();
        self.worker_pass();
    }

    fn run_inner(&mut self) {
        if let Some(relay) = self.relay.as_mut() {
            relay.start(0, &mut self.host);
        }
        let mut first = true;
        loop {
            if self.guest.completed >= self.sc.flips && self.now >= self.sc.min_sim_ns {
                break;
            }
            let Some(t) = self.next_event() else { break };
            if t > self.sc.max_ns || (self.sc.fixed_dt.is_some() && t >= self.sc.max_ns) {
                break;
            }
            if t == self.now && !first {
                self.stepped_same += 1;
                if self.stepped_same > 20_000 {
                    self.fail("LIVELOCK: 20000 steps without time advancing");
                }
            } else {
                self.stepped_same = 0;
            }
            first = false;
            self.now = self.now.max(t);
            if self.sc.fixed_dt.is_some() && self.base.is_none() && self.now >= 1_000_000_000 {
                self.base = Some(Base {
                    completed: self.guest.completed,
                    copies: self.mdl.copies,
                    ticks: self.out.ticks,
                    skipped: self.mdl.mailbox.skipped,
                    presents: self.e.pace[0].presents,
                    tearing: self.e.pace[0].tearing,
                });
            }
            self.step();
            self.check();
            self.out.steps += 1;
        }
    }

    /// Drain what the broker pushed (no time passes), then the scenario's end-of-run checks.
    fn drain(&mut self) {
        for _ in 0..100_000 {
            let go = {
                let w = self.wire.borrow();
                self.relay.as_ref().is_some_and(Relay::connected) && !w.inbox.is_empty()
            };
            if !go {
                break;
            }
            self.relay_entry(Entry::Socket { cap: READ_BATCH });
        }
    }

    fn run(mut self) -> Out {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.run_inner()));
        if let Err(e) = r {
            let msg = e
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_string()))
                .unwrap_or_default();
            if !msg.contains("lockstep_soak FAILED") {
                eprintln!(
                    "\nlockstep_soak: scenario `{}` seed {} PANICKED at t={} ms: {msg}\n{}replay: KF_SOAK_SEED={}",
                    self.sc.name,
                    self.seed,
                    self.now / MS,
                    self.dump(),
                    self.seed
                );
            }
            std::panic::resume_unwind(e);
        }
        if self.sc.exact || self.sc.relay {
            self.drain();
        }
        self.finish()
    }

    fn finish(mut self) -> Out {
        let o = &mut self.out;
        o.sim_ns = self.now;
        o.submitted = self.guest.submitted;
        o.completed = self.guest.completed;
        o.imm = self.guest.imm;
        o.copies = self.mdl.copies;
        o.skipped = self.mdl.mailbox.skipped;
        o.latches = self.mdl.latches;
        o.mailbox_coalesced = self.mdl.mailbox.coalesced;
        o.gate = self.gate.c;
        o.handoffs = self.pacer.source(0).map_or((0, 0), |s| s.handoffs());
        o.probation = self.pacer.source(0).map_or(0, |s| s.probation_events());
        o.commits = self.broker.commits;
        o.answered = self.broker.answered;
        o.unsolicited = self.broker.unsolicited;
        o.no_changes = self.broker.no_changes;
        o.ticks_pushed = self.broker.ticks_pushed;
        o.stale_pushed = self.broker.stale_pushed;
        o.noise_pushed = self.broker.noise_pushed;
        o.connects = self.wire.borrow().connects;
        o.hostile = self.out_hostile;
        o.goal = self.sc.flips;
        o.long_gaps = self.broker.pattern_long_gaps;
        o.relay_on = self.sc.relay;
        o.advertised = self.sc.b.advertise;
        o.presents = self.e.pace[0].presents;
        o.tearing = self.e.pace[0].tearing;
        o.base = self.base;
        o.sock_full_drops = self.wire.borrow().sock_full_drops;
        o.fulls = self.wire.borrow().fulls;
        if let Some(r) = &self.relay {
            o.relay = r.tick_counters();
            o.relay_c = r.counters();
        }
        if o.min_broker_gap_ns == u64::MAX {
            o.min_broker_gap_ns = 0;
        }
        let o = self.out.clone();
        if self.sc.exact && self.relay.is_some() && o.lost == 0 && o.connects == 1 {
            let b = &self.broker;
            let r = &o.relay;
            let want = (b.e_ticks, b.e_stale, b.e_missed, b.e_want, b.e_back);
            let got = (r.ticks, r.ticks_stale, r.ticks_missed, r.want_ticks, r.ts_backwards);
            if want != got {
                self.fail(&format!(
                    "the relay's tick counters (ticks, stale, missed, want, backwards) are {got:?}, the broker expected {want:?}"
                ));
            }
        }
        o
    }
}

// ------------------------------------------------------------------------------------------------
// the runner and the scenarios

const SEEDS: u64 = 50;

fn seeds() -> Vec<u64> {
    match std::env::var("KF_SOAK_SEED").ok().and_then(|s| s.parse().ok()) {
        Some(s) => vec![s],
        None => (0..SEEDS).collect(),
    }
}

/// Run `mk(seed)` for every seed; print one summary line (flips, simulated seconds, wall time).
fn soak(name: &str, mk: impl Fn(u64) -> Sc) -> Vec<Out> {
    let t0 = std::time::Instant::now();
    let outs: Vec<Out> = seeds().into_iter().map(|s| World::new(mk(s), s).run()).collect();
    let flips: u64 = outs.iter().map(|o| o.completed).sum();
    let sim: u64 = outs.iter().map(|o| o.sim_ns).sum();
    let steps: u64 = outs.iter().map(|o| o.steps).sum();
    eprintln!(
        "lockstep_soak {name}: {} seeds, {flips} flips completed, {:.1} simulated s, {steps} steps, {} ms wall",
        outs.len(),
        sim as f64 / 1e9,
        t0.elapsed().as_millis()
    );
    outs
}

fn total<F: Fn(&Out) -> u64>(o: &[Out], f: F) -> u64 {
    o.iter().map(f).sum()
}

fn flips_goal(n: u64) -> u64 {
    std::env::var("KF_SOAK_FLIPS").ok().and_then(|s| s.parse().ok()).unwrap_or(n)
}

/// The common assertions of every scenario with a live relay: the goal was reached, a COMMIT is an
/// answer to exactly one request (or an unsolicited fallback frame, or a replay), nothing is lost
/// from the books.
fn books(o: &Out) {
    assert!(
        o.completed >= o.goal,
        "seed {}: only {} of {} flips completed in {} s of simulated time",
        o.seed,
        o.completed,
        o.goal,
        o.sim_ns / 1_000_000_000
    );
    if o.relay_on && o.advertised {
        assert_eq!(
            o.answered, o.relay.frames_on_credit,
            "seed {}: frames answered (broker) vs spent requests (relay)",
            o.seed
        );
        // every COMMIT is an answer, an unsolicited fallback frame, or the replay of a reconnect
        assert!(
            o.commits <= o.answered + o.unsolicited + u64::from(o.connects),
            "seed {}: {} commits, {} answered, {} unsolicited, {} connections",
            o.seed,
            o.commits,
            o.answered,
            o.unsolicited,
            o.connects
        );
    }
}

fn base_sc(name: &'static str) -> Sc {
    let mut sc = Sc::new(name);
    sc.flips = flips_goal(2000);
    sc
}

/// (1) The guest renders faster than the display refreshes, immediate flips only: every flip
/// latches at once, the copies and the frames are bounded by the broker's requests, the newest wins.
#[test]
fn lockstep_soak_s1_fast_guest_immediate() {
    let outs = soak("s1 fast guest, immediate", |s| {
        let mut sc = base_sc("lockstep_soak_s1_fast_guest_immediate");
        sc.render = (100_000, 2 * MS + (s % 3) * MS);
        sc.exact = true;
        sc
    });
    for o in &outs {
        books(o);
        assert_eq!(o.imm, o.submitted, "seed {}", o.seed);
        assert!(o.copies <= o.broker_ticks + 2, "seed {}: {} copies, {} ticks", o.seed, o.copies, o.broker_ticks);
        assert!(o.skipped > o.latches / 4, "seed {}: newest wins, the rest are skipped: {o:?}", o.seed);
        assert_eq!(o.free_ticks, 0, "seed {}: the broker ticks the whole run", o.seed);
        assert_eq!(o.handoffs, (1, 0), "seed {}", o.seed);
        assert!(o.relay.credits_coalesced == 0 || o.relay.want_ticks > 0);
    }
    let fps: Vec<f64> = outs.iter().map(|o| o.completed as f64 / (o.sim_ns as f64 / 1e9)).collect();
    eprintln!("  s1 flips/s per seed: min {:.0} max {:.0} (display ticks at 60)", fps.iter().cloned().fold(f64::MAX, f64::min), fps.iter().cloned().fold(0.0, f64::max));
    assert!(fps.iter().all(|f| *f > 150.0), "{fps:?}");
}

/// (2) Two flips per frame, one immediate and one vsync (phase 1's inferred Windows pattern): the
/// immediate flip is never parked, the vsync one completes at a tick, about 120 flips a second.
#[test]
fn lockstep_soak_s2_two_flips_per_frame() {
    let outs = soak("s2 immediate + vsync", |_| {
        let mut sc = base_sc("lockstep_soak_s2_two_flips_per_frame");
        sc.gk = GKind::Alt;
        sc.render = (MS, 4 * MS);
        sc.exact = true;
        sc
    });
    for o in &outs {
        books(o);
        assert!(o.imm.abs_diff(o.submitted / 2) <= 1, "seed {}: {} of {}", o.seed, o.imm, o.submitted);
        let fps = o.completed as f64 / (o.sim_ns as f64 / 1e9);
        assert!((90.0..=125.0).contains(&fps), "seed {}: {fps:.1} flips/s", o.seed);
        assert!(o.worst_flip_ns <= FLIP_BOUND, "seed {}", o.seed);
    }
}

/// (3) The broker steady at the refresh, a guest of every speed: the books balance exactly, one
/// request per frame, the relay's counters equal the broker's expectations.
#[test]
fn lockstep_soak_s3_broker_steady_at_refresh() {
    let outs = soak("s3 steady", |s| {
        let mut sc = base_sc("lockstep_soak_s3_broker_steady_at_refresh");
        sc.exact = true;
        sc.gk = [GKind::Imm, GKind::Vsync, GKind::Alt, GKind::Rand][(s % 4) as usize];
        sc.render = (MS, 12 * MS);
        sc.b.noisy = s % 2 == 1;
        sc.b.jitter = if s % 3 == 0 { 2 * MS } else { 0 };
        sc.burst_pm = 30;
        sc.pause_pm = 5;
        sc.caps = vec![1, 7, READ_BATCH];
        sc
    });
    for o in &outs {
        books(o);
        assert_eq!(o.relay.ticks, o.ticks_pushed, "seed {}", o.seed);
        assert_eq!(o.lost, 0, "seed {}", o.seed);
        assert!(o.answered > 0, "seed {}", o.seed);
        assert!(o.min_broker_gap_ns >= P60 / 2, "seed {}", o.seed);
    }
}

/// (4) The broker TOO FAST: tick floods of 10x, 100x and 1000x the refresh, with `WANT_FRAME` on every
/// tick and on none, steady and in bursts of 32, in windows of 50-150 ms every half second, with every
/// other message kind interleaved (RELEASE, EV_FRAME, pointer/focus/surface/device state, unknown
/// types). Nothing starves, nothing blocks, work per turn is bounded, the vblank never exceeds twice the
/// refresh (the guard), at most one request is open, the counters are exact.
#[test]
fn lockstep_soak_s4_floods() {
    let outs = soak("s4 floods", |s| {
        let combo = s % 12;
        let mult = [10u32, 100, 1000][(combo % 3) as usize];
        let want = if (combo / 3) % 2 == 0 { WantPolicy::Always } else { WantPolicy::Never };
        let burst = if combo / 6 == 0 { 1 } else { 32 };
        let mut sc = base_sc("lockstep_soak_s4_floods");
        sc.exact = true;
        sc.b.noisy = true;
        sc.b.want = want;
        sc.render = (MS, 6 * MS);
        sc.burst_pm = 20;
        sc.caps = vec![1, 16, READ_BATCH];
        for k in 0..80u64 {
            let t = 300 * MS + k * 500 * MS;
            sc.timeline.push((t, Act::Mode(Mode::Flood { mult, burst })));
            sc.timeline.push((t + (50 + (k % 5) * 25) * MS, Act::Mode(Mode::Steady)));
        }
        sc
    });
    let mut rates = Vec::new();
    for o in &outs {
        books(o);
        assert!(o.max_turn_packets <= READ_BATCH as u64, "seed {}", o.seed);
        let cap = o.sim_ns / (P60 / 2) + 3;
        assert!(o.ticks <= cap, "seed {}: {} vblanks in {} ms exceed the guard's cap {cap}", o.seed, o.ticks, o.sim_ns / MS);
        assert!(o.relay.budget_hits > 0 || o.ticks_pushed < 50, "seed {}: a flood must hit the per-turn budget", o.seed);
        assert!(o.max_held <= 2);
        assert_eq!(o.relay.ticks + o.relay.ticks_stale, o.ticks_pushed, "seed {}", o.seed);
        rates.push(o.ticks as f64 / (o.sim_ns as f64 / 1e9));
    }
    let max = rates.iter().cloned().fold(0.0, f64::max);
    eprintln!(
        "  s4: tick pushes {}, relay ticks {}, socket-full drops (broker side) {}, relay budget hits {}, credits coalesced {}, worker coalesced ticks {}; max vblank rate {max:.1}/s (mode 60)",
        total(&outs, |o| o.ticks_pushed),
        total(&outs, |o| o.relay.ticks),
        total(&outs, |o| o.sock_full_drops),
        total(&outs, |o| o.relay.budget_hits),
        total(&outs, |o| o.relay.credits_coalesced),
        total(&outs, |o| o.gate.ticks_coalesced),
    );
}

/// (4b) Constant floods at 10x / 100x / 1000x the refresh for the whole run — the measured drop counts
/// of the report. A flood with no WANT, then one with a WANT on every tick.
#[test]
fn lockstep_soak_flood_rates() {
    for mult in [10u32, 100, 1000] {
        for want in [WantPolicy::Never, WantPolicy::Always] {
            let name = "lockstep_soak_flood_rates";
            let outs = soak(&format!("flood x{mult} {want:?}"), |s| {
                let mut sc = Sc::new(name);
                sc.flips = flips_goal(if mult == 1000 { 150 } else { 600 });
                sc.exact = true;
                sc.b.noisy = true;
                sc.b.want = want;
                sc.mode0 = Mode::Flood { mult, burst: if s % 2 == 0 { 1 } else { 200 } };
                sc.turn_gap_ns = 5 * MS;
                sc.render = (2 * MS, 8 * MS);
                sc.caps = vec![READ_BATCH];
                sc
            });
            for o in &outs {
                assert!(o.completed >= o.submitted.saturating_sub(1), "seed {}", o.seed);
                assert!(o.max_turn_packets <= READ_BATCH as u64);
                assert!(o.ticks <= o.sim_ns / (P60 / 2) + 3, "seed {}", o.seed);
                assert_eq!(o.relay.ticks + o.relay.ticks_stale, o.ticks_pushed);
                assert!(
                    !(mult == 1000 || o.seed % 2 == 1) || o.relay.budget_hits > 0,
                    "seed {}: the per-turn budget must bind",
                    o.seed
                );
            }
            let secs: f64 = outs.iter().map(|o| o.sim_ns as f64 / 1e9).sum();
            eprintln!(
                "  flood x{mult} {want:?}: {:.0} tick pushes/s (offered {:.0}/s), {} broker-side socket-full drops, relay ticks {}, stale {}, credits coalesced {}, worker saw-latest-only {}, vblanks {:.1}/s, budget hits {}, no_change {}",
                total(&outs, |o| o.ticks_pushed) as f64 / secs,
                f64::from(mult) * 60.0,
                total(&outs, |o| o.sock_full_drops),
                total(&outs, |o| o.relay.ticks),
                total(&outs, |o| o.relay.ticks_stale),
                total(&outs, |o| o.relay.credits_coalesced),
                total(&outs, |o| o.gate.ticks_coalesced),
                total(&outs, |o| o.ticks) as f64 / secs,
                total(&outs, |o| o.relay.budget_hits),
                total(&outs, |o| o.no_changes),
            );
        }
    }
}

/// (5) Early, duplicated, stale, reordered, dropped ticks and timestamps that go backwards: stale
/// numbers are dropped and counted EXACTLY, never acted on, never keep the broker 'alive'; no double
/// tick; one frame per request.
#[test]
fn lockstep_soak_s5_early_and_out_of_sequence_ticks() {
    let outs = soak("s5 early / out of sequence", |s| {
        let mut sc = base_sc("lockstep_soak_s5_early_and_out_of_sequence_ticks");
        sc.exact = true;
        sc.b.period = [P60, P60 * 45 / 100, P60 * 7 / 10][(s % 3) as usize];
        sc.b.jitter = 3 * MS;
        sc.b.dup = 150;
        sc.b.stale = 150;
        sc.b.reorder = 120;
        sc.b.drop = 60;
        sc.b.backwards = 80;
        sc.b.noisy = s % 2 == 0;
        sc.b.want = if s % 5 == 0 { WantPolicy::Always } else { WantPolicy::Lockstep };
        sc.render = (MS, 6 * MS);
        sc.gk = [GKind::Imm, GKind::Alt][(s % 2) as usize];
        sc.caps = vec![1, 3, READ_BATCH];
        sc
    });
    for o in &outs {
        books(o);
        assert!(o.relay.ticks_stale > 0 && o.stale_pushed == o.relay.ticks_stale, "seed {}", o.seed);
        assert!(o.relay.ticks_missed > 0 && o.relay.ts_backwards > 0, "seed {}: {:?}", o.seed, o.relay);
        assert_eq!(o.gate.stale, 0, "seed {}: the relay filters stale ticks before the worker sees them", o.seed);
    }
    eprintln!(
        "  s5: stale dropped {} (all counted), missed {}, backwards {}, credits coalesced {}",
        total(&outs, |o| o.relay.ticks_stale),
        total(&outs, |o| o.relay.ticks_missed),
        total(&outs, |o| o.relay.ts_backwards),
        total(&outs, |o| o.relay.credits_coalesced)
    );
}

/// (6) The broker goes silent for 49..1000 ms and comes back, with copies long enough to be in flight
/// when it happens: the clock takes over (the vblank never stalls), frames keep flowing on the legacy
/// credit, the broker gets the tick back once, a stale burst after the silence changes nothing.
#[test]
fn lockstep_soak_s6_silence_then_return_mid_copy() {
    let durs = [49u64, 50, 51, 60, 200, 1000];
    let outs = soak("s6 silence / return", |s| {
        let mut sc = base_sc("lockstep_soak_s6_silence_then_return_mid_copy");
        sc.copy = (6 * MS, 20 * MS);
        sc.render = (200_000, 4 * MS);
        sc.exact = true;
        for k in 0..6u64 {
            let d = durs[((s + k) % 6) as usize] * MS;
            let t = 1000 * MS + k * 3000 * MS;
            sc.timeline.push((t, Act::Mode(Mode::Silent)));
            sc.timeline.push((t + d, Act::Resume(8)));
            sc.timeline.push((t + d, Act::Mode(Mode::Steady)));
        }
        sc.min_sim_ns = 20 * 1000 * MS;
        sc.flips = flips_goal(2000);
        sc
    });
    for o in &outs {
        books(o);
        let seed = o.seed;
        let falls = (0..6u64).filter(|k| durs[((seed + k) % 6) as usize] >= 50).count() as u64;
        assert!(o.handoffs.1 >= falls, "seed {seed}: {} fallbacks, expected at least {falls}", o.handoffs.1);
        assert!(o.handoffs.0 >= 2, "seed {seed}: the broker must get the tick back: {:?}", o.handoffs);
        assert!(o.handoffs.0 <= o.handoffs.1 + 1, "seed {seed}: one handoff back per fallback: {:?}", o.handoffs);
        assert!(o.free_ticks > 0, "seed {seed}");
        assert!(o.unsolicited > 0, "seed {seed}: frames must keep flowing in the fallback (the legacy credit)");
        assert!(o.relay.silence_fallbacks >= 1 && o.relay.silence_resumes >= 2, "seed {seed}: {:?}", o.relay);
        assert!(o.stale_pushed >= 8, "seed {seed}");
    }
}

/// (7) HEADLESS, and a broker that never connects: the display plane behaves bit for bit as phase 1
/// (`pacing_model::run`): same flips, presents, copies, ticks, skipped, worst flip, no handoff.
#[test]
fn lockstep_soak_s7_headless_is_bit_identical_to_phase_1() {
    use super::pacing_model::{Cfg, Kind as PKind, run as phase1};
    for (kind, pk) in [
        (GKind::Imm, PKind::Immediate),
        (GKind::Vsync, PKind::Vsync),
        (GKind::Alt, PKind::Alternate),
    ] {
        for buffers in [2usize, 3] {
            for r in [1.0f64, 4.0, 12.0] {
                let rep = phase1(Cfg::new(buffers, r, pk), None, 4);
                for (relay, up) in [(false, false), (true, false)] {
                    let mut sc = Sc::new("lockstep_soak_s7_headless");
                    sc.gk = kind;
                    sc.buffers = Some(buffers);
                    sc.render = ((r * MS as f64) as u64, (r * MS as f64) as u64);
                    sc.copy = (2 * MS, 2 * MS);
                    sc.wake = MS / 2;
                    sc.relay = relay;
                    sc.broker_up = up;
                    sc.fixed_dt = Some(25_000);
                    sc.max_ns = 5_000_000_000;
                    sc.flips = u64::MAX;
                    let o = World::new(sc, 1).run();
                    let b = o.base.expect("the measured window began");
                    let sec = 4.0f64;
                    let ctx = format!("{kind:?} {buffers} buffers R={r} relay={relay}");
                    assert_eq!((o.completed - b.completed) as f64 / sec, rep.flips_per_s, "{ctx}: flips/s");
                    assert_eq!((o.presents - b.presents) as f64 / sec, rep.presents_per_s, "{ctx}: presents/s");
                    assert_eq!((o.tearing - b.tearing) as f64 / sec, rep.tear_per_s, "{ctx}: tear/s");
                    assert_eq!((o.copies - b.copies) as f64 / sec, rep.copies_per_s, "{ctx}: copies/s");
                    assert_eq!((o.ticks - b.ticks) as f64 / sec, rep.ticks_per_s, "{ctx}: ticks/s");
                    assert_eq!((o.skipped - b.skipped) as f64 / sec, rep.skipped_per_s, "{ctx}: skipped/s");
                    assert_eq!(o.worst_flip_ns as f64 / 1e6, rep.worst_flip_ms, "{ctx}: worst flip");
                    assert_eq!(rep.violations, 0);
                    assert_eq!(o.handoffs, rep.handoffs, "{ctx}");
                    assert_eq!(o.handoffs, (0, 0), "{ctx}: nothing feeds a broker event");
                    assert_eq!(o.gate, crate::lockstep::GateCounters::default(), "{ctx}: the gate is untouched");
                    assert_eq!(o.broker_ticks, 0, "{ctx}");
                    assert_eq!(o.relay.ticks, 0, "{ctx}");
                }
            }
        }
    }
    // and one recorded run, so the reference itself cannot drift unnoticed
    let rep = phase1(Cfg::new(2, 4.0, PKind::Immediate), None, 4);
    eprintln!("  s7 recorded phase-1 run (2 buffers, R=4 ms, immediate): {rep:?}");
}

/// (8) The broker crashes: mid-frame (the moment an ATTACH arrives), while the socket is full, plain.
/// The guest never notices; the relay reconnects; the book of the new connection starts clean; the
/// broker takes the tick back after every reconnect (the worker resets its gate at a new connection).
#[test]
fn lockstep_soak_s8_broker_crash_mid_frame() {
    let outs = soak("s8 crash", |s| {
        let mut sc = base_sc("lockstep_soak_s8_broker_crash_mid_frame");
        sc.render = (MS, 6 * MS);
        sc.copy = (MS, 10 * MS);
        sc.b.noisy = s % 2 == 0;
        for k in 0..8u64 {
            let b = 1000 * MS + k * 2500 * MS;
            match (k + s) % 4 {
                0 => {
                    sc.timeline.push((b, Act::CrashOnAttach));
                    sc.timeline.push((b + 600 * MS, Act::Up));
                }
                1 => {
                    sc.timeline.push((b, Act::NeverRead(true)));
                    sc.timeline.push((b + 40 * MS, Act::Down));
                    sc.timeline.push((b + 700 * MS, Act::Up));
                    sc.timeline.push((b + 710 * MS, Act::NeverRead(false)));
                }
                2 => {
                    sc.timeline.push((b, Act::Down));
                    sc.timeline.push((b + 300 * MS, Act::Up));
                }
                _ => {
                    sc.timeline.push((b, Act::Hung(true)));
                    sc.timeline.push((b + 30 * MS, Act::Down));
                    sc.timeline.push((b + 500 * MS, Act::Hung(false)));
                    sc.timeline.push((b + 510 * MS, Act::Up));
                }
            }
        }
        sc.min_sim_ns = 22 * 1000 * MS;
        sc
    });
    for o in &outs {
        books(o);
        assert!(o.connects >= 5, "seed {}: only {} connections", o.seed, o.connects);
        assert!(o.lost >= 4, "seed {}", o.seed);
        assert!(o.handoffs.0 >= 3, "seed {}: the broker must get the tick back after reconnects: {:?}", o.seed, o.handoffs);
        assert_eq!(o.gate.stale, 0, "seed {}: no restarted numbering may look stale to the gate", o.seed);
    }
}

/// ★ DEFECT (found by scenario 8, reported to the owner of the code; this test pins the CURRENT
/// behaviour of `display.rs`, which resets the credit gate only when a guest life ends): after a broker
/// restart the new connection's tick numbers start over at 1, the worker's gate still remembers the
/// old connection's last number, calls every new tick STALE and ignores the broker's ticks and
/// requests until the numbers pass the old one. The guest is not harmed (the clock ticks and flips
/// complete); the lock step is silently off for as long as the old connection had been up.
/// When the worker resets the gate at a new connection this test must be changed to expect `stale == 0`
/// (the world's `reset_gate`, exercised by scenario 8).
#[test]
fn lockstep_soak_defect_gate_not_reset_at_a_new_connection() {
    let outs = soak("defect: gate not reset", |_| {
        let mut sc = base_sc("lockstep_soak_defect_gate_not_reset_at_a_new_connection");
        sc.reset_gate = false;
        sc.render = (MS, 4 * MS);
        sc.timeline = vec![(3000 * MS, Act::Down), (3300 * MS, Act::Up)];
        sc.min_sim_ns = 9000 * MS;
        sc
    });
    for o in &outs {
        assert!(o.connects >= 2, "seed {}", o.seed);
        assert!(
            o.gate.stale >= 30,
            "seed {}: expected the defect (stale events after the reconnect), got {:?}",
            o.seed,
            o.gate
        );
        assert!(o.completed >= 2000, "the guest is not harmed");
    }
    eprintln!(
        "  defect: after a restart at 3.3 s the gate called {} broker ticks stale ({} per seed)",
        total(&outs, |o| o.gate.stale),
        total(&outs, |o| o.gate.stale) / outs.len() as u64
    );
}

/// (9) Hostile framing: every violation of the VMM-side rows of §11.2 drops the connection (and only
/// that); the guest keeps running; the relay reconnects.
#[test]
fn lockstep_soak_s9_hostile_framing() {
    let outs = soak("s9 hostile framing", |s| {
        let mut sc = base_sc("lockstep_soak_s9_hostile_framing");
        sc.hostile = true;
        sc.render = (MS, 5 * MS);
        sc.b.advertise = s % 2 == 0;
        let kinds: &[u8] = if sc.b.advertise { &[0, 1, 2, 3, 9] } else { &[5, 5, 9, 4] };
        for k in 0..12u64 {
            sc.timeline.push((1000 * MS + k * 900 * MS, Act::Hostile(kinds[(k as usize + s as usize) % kinds.len()])));
        }
        sc.min_sim_ns = 13_000 * MS;
        sc
    });
    for o in &outs {
        books(o);
        assert!(o.hostile >= 8, "seed {}: {} hostile packets", o.seed, o.hostile);
        assert!(o.lost >= o.hostile, "seed {}: {} violations sent, {} connections dropped", o.seed, o.hostile, o.lost);
        assert!(o.connects as u64 > o.hostile.saturating_sub(1), "seed {}: the relay must reconnect after each", o.seed);
    }
    eprintln!("  s9: {} violations sent, {} connections dropped", total(&outs, |o| o.hostile), total(&outs, |o| o.lost));
}

/// ★ The adversarial scheduler: everything at once — floods, silences of every length, SIGSTOP, a
/// socket that stops draining, restarts, partial packets, hostile packets, stale/duplicated/reordered/
/// dropped ticks, releases withheld, guest bursts and pauses — for >= 100 000 flips over 50 seeds.
#[test]
fn lockstep_soak_adversarial() {
    let outs = soak("adversarial", adversarial_sc);
    for o in &outs {
        books(o);
        assert!(o.max_turn_packets <= READ_BATCH as u64);
        assert!(o.max_held <= 2);
    }
    summary_adv(&outs);
}

fn adversarial_sc(s: u64) -> Sc {
    let mut sc = base_sc("lockstep_soak_adversarial");
    sc.chaos = true;
    sc.gk = [GKind::Imm, GKind::Alt, GKind::Rand, GKind::Vsync][(s % 4) as usize];
    sc.render = (200_000, 8 * MS);
    sc.copy = (500_000, 15 * MS);
    sc.burst_pm = 40;
    sc.pause_pm = 8;
    sc.b.noisy = true;
    sc.b.jitter = 3 * MS;
    sc.b.dup = 20;
    sc.b.stale = 30;
    sc.b.reorder = 20;
    sc.b.drop = 10;
    sc.b.backwards = 10;
    sc.caps = vec![1, 4, 16, READ_BATCH];
    sc
}

fn summary_adv(outs: &[Out]) {
    eprintln!(
        "  adversarial: flips {}, connections {}, dropped {}, fallbacks {}, handoffs {:?}, stale {}, coalesced credits {}, budget hits {}, no_change {}, unsolicited {}, socket-full drops {}, full sends {}",
        total(outs, |o| o.completed),
        total(outs, |o| u64::from(o.connects)),
        total(outs, |o| o.lost),
        total(outs, |o| o.relay.silence_fallbacks),
        (total(outs, |o| o.handoffs.0), total(outs, |o| o.handoffs.1)),
        total(outs, |o| o.relay.ticks_stale),
        total(outs, |o| o.relay.credits_coalesced),
        total(outs, |o| o.relay.budget_hits),
        total(outs, |o| o.no_changes),
        total(outs, |o| o.unsolicited),
        total(outs, |o| o.sock_full_drops),
        total(outs, |o| o.fulls),
    );
}

/// The long soak: `KF_SOAK_FLIPS` flips in all (10 000 000 by default) over the adversarial scheduler.
#[test]
#[ignore = "long soak: 10M flips (minutes)"]
fn lockstep_soak_long() {
    let total_flips: u64 = std::env::var("KF_SOAK_FLIPS").ok().and_then(|s| s.parse().ok()).unwrap_or(10_000_000);
    let per = total_flips / SEEDS;
    let outs = soak("long", |s| {
        let mut sc = adversarial_sc(s);
        sc.flips = per;
        sc
    });
    for o in &outs {
        books(o);
    }
    summary_adv(&outs);
}

// ------------------------------------------------------------------------------------------------
// the broker STALLS (distinct from silence and from floods)

/// Stall durations, milliseconds: below, on and above the 50 ms boundary, then long.
const STALLS_MS: [u64; 7] = [10, 49, 50, 51, 200, 2000, 10_000];

fn stall_dur(seed: u64) -> u64 {
    STALLS_MS[(seed % 7) as usize]
}

/// (st1) The broker stops READING (the socket is full) but still sends ticks, for 10 ms .. 10 s: frames
/// are dropped and counted, nothing blocks (the link panics on a retried full send), the ticks keep
/// driving the vblank (no fallback), and the stream recovers when the broker reads again.
#[test]
fn lockstep_soak_st1_broker_stops_reading_but_still_ticks() {
    let outs = soak("st1 stops reading", |s| {
        let d = stall_dur(s) * MS;
        let mut sc = base_sc("lockstep_soak_st1_broker_stops_reading_but_still_ticks");
        sc.render = (MS, 5 * MS);
        sc.timeline = vec![(3000 * MS, Act::NeverRead(true)), (3000 * MS + d, Act::NeverRead(false))];
        sc.min_sim_ns = 3000 * MS + d + 3000 * MS;
        sc.b.noisy = s % 2 == 0;
        sc
    });
    for o in &outs {
        books(o);
        let d = stall_dur(o.seed);
        assert_eq!(o.handoffs.1, 0, "seed {}: the ticks never stopped, the clock must not take over", o.seed);
        assert!(o.max_tick_gap_ns <= P60 + MS, "seed {}: vblank gap {} ns", o.seed, o.max_tick_gap_ns);
        if d >= 200 {
            assert!(o.fulls > 0 && o.relay_c.dropped > 0, "seed {}: frames must be dropped and counted: {:?}", o.seed, o.relay_c);
        }
        assert!(o.sim_ns - o.last_commit_ns < 1500 * MS, "seed {}: the frame stream did not recover", o.seed);
        assert!(o.max_held <= 2);
    }
    eprintln!(
        "  st1: durations {STALLS_MS:?} ms; frames dropped (ATTACH met a full socket) {}, full sends {}, uncommitted {}",
        total(&outs, |o| o.relay_c.dropped),
        total(&outs, |o| o.fulls),
        total(&outs, |o| o.relay_c.uncommitted)
    );
}

/// (st1b) A broker that NEVER reads (from the start, or from 1 s): no frame is ever delivered, nothing
/// blocks, the guest runs for ever, the relay reports it (dropped frames, failed attempts), memory
/// stays bounded.
#[test]
fn lockstep_soak_st1b_a_broker_that_never_reads() {
    let outs = soak("st1b never reads", |s| {
        let mut sc = base_sc("lockstep_soak_st1b_a_broker_that_never_reads");
        sc.render = (MS, 5 * MS);
        let at = if s % 2 == 0 { 0 } else { 1000 * MS };
        sc.timeline = vec![(at, Act::NeverRead(true))];
        sc.min_sim_ns = 6000 * MS;
        sc
    });
    for o in &outs {
        assert!(o.completed >= o.goal, "seed {}: the guest must not notice", o.seed);
        assert!(o.fulls > 0, "seed {}", o.seed);
        if o.seed % 2 == 1 {
            assert!(o.relay_c.dropped > 0, "seed {}: {:?}", o.seed, o.relay_c);
        } else {
            assert!(o.relay_c.attempts_failed >= 1 || o.connects == 0, "seed {}: a replay into a full socket fails the attempt: {:?}", o.seed, o.relay_c);
        }
        assert!(o.max_held <= 2);
    }
}

/// (st2) The broker stops sending ticks but still reads and holds frames, 10 ms .. 10 s: past 50 ms of
/// silence the clock takes over (the vblank never stops), frames flow only on the legacy credit (no frame
/// without a credit is asserted at every step), the broker gets the tick back once.
#[test]
fn lockstep_soak_st2_broker_stops_ticking_but_still_reads() {
    let log = kf_broker::capture_log();
    let outs = soak("st2 stops ticking", |s| {
        let d = stall_dur(s) * MS;
        let mut sc = base_sc("lockstep_soak_st2_broker_stops_ticking_but_still_reads");
        sc.render = (MS, 5 * MS);
        sc.timeline = vec![
            (3000 * MS, Act::Mode(Mode::Silent)),
            (3000 * MS + d, Act::Mode(Mode::Steady)),
            (3000 * MS + d, Act::Resume(12)),
        ];
        sc.min_sim_ns = 3000 * MS + d + 2000 * MS;
        sc.exact = true;
        sc
    });
    for o in &outs {
        books(o);
        let d = stall_dur(o.seed);
        if d >= 100 {
            assert!(o.handoffs.1 >= 1 && o.relay.silence_fallbacks >= 1, "seed {}: {:?} {:?}", o.seed, o.handoffs, o.relay);
            assert_eq!(o.handoffs.0, o.handoffs.1 + 1, "seed {}: the broker gets the tick back exactly once per fallback", o.seed);
            assert!(o.unsolicited >= 1 || d < 200, "seed {}: frames must flow on the legacy credit", o.seed);
        }
        assert!(o.handoffs.0 <= o.handoffs.1 + 1);
    }
    let lines = log.lines();
    assert!(lines.iter().any(|l| l.contains("SILENT")), "the stall must be visible in the log");
    eprintln!(
        "  st2: durations {STALLS_MS:?} ms; fallbacks {}, handoffs {:?}, unsolicited frames {}, relay log lines {} (SILENT/LIVE: {})",
        total(&outs, |o| o.handoffs.1),
        (total(&outs, |o| o.handoffs.0), total(&outs, |o| o.handoffs.1)),
        total(&outs, |o| o.unsolicited),
        lines.len(),
        lines.iter().filter(|l| l.contains("SILENT") || l.contains("LIVE")).count()
    );
}

/// (st3) The broker never RELEASES a buffer: the held frames stay two, the slot a copy fills is never a
/// held or ready one (checked at every copy), the relay blocks and counts, reclaims after a second, and
/// nothing deadlocks.
#[test]
fn lockstep_soak_st3_broker_never_releases_a_slot() {
    let outs = soak("st3 never releases", |s| {
        let d = [300u64, 1000, 2000, 6000][(s % 4) as usize] * MS;
        let mut sc = base_sc("lockstep_soak_st3_broker_never_releases_a_slot");
        sc.render = (MS, 4 * MS);
        sc.timeline = vec![(2000 * MS, Act::NoRelease(true)), (2000 * MS + d, Act::NoRelease(false))];
        sc.min_sim_ns = 2000 * MS + d + 2500 * MS;
        sc
    });
    for o in &outs {
        books(o);
        let d = [300u64, 1000, 2000, 6000][(o.seed % 4) as usize];
        assert!(o.max_held <= 2, "seed {}", o.seed);
        assert!(o.relay_c.blocked > 0, "seed {}: {:?}", o.seed, o.relay_c);
        if d >= 2000 {
            assert!(o.relay_c.reclaims > 0, "seed {}: an unreleased frame must be reclaimed after a second", o.seed);
        }
        assert!(o.sim_ns - o.last_commit_ns < 2000 * MS, "seed {}: no deadlock", o.seed);
        assert!(o.worst_flip_ns <= FLIP_BOUND, "seed {}: the guest is not held back by a slot the broker keeps", o.seed);
    }
    eprintln!("  st3: blocked {}, reclaimed {}", total(&outs, |o| o.relay_c.blocked), total(&outs, |o| o.relay_c.reclaims));
}

/// (st4) The broker stalls MID-PACKET (a partial tick, then nothing): the partial state is one packet
/// (24 bytes), the connection is dropped LOUDLY at the named read deadline (500 ms), no thread blocks;
/// a partial packet that completes in time is just a packet.
#[test]
fn lockstep_soak_st4_broker_stalls_mid_message() {
    let log = kf_broker::capture_log();
    let outs = soak("st4 partial packet", |s| {
        let mut sc = base_sc("lockstep_soak_st4_broker_stalls_mid_message");
        sc.render = (MS, 5 * MS);
        let n = 1 + (s % 23) as usize;
        sc.timeline = vec![(2000 * MS, Act::Partial(n))];
        if s % 2 == 1 {
            sc.timeline.push((2000 * MS + 100 * MS, Act::Complete));
        }
        sc.min_sim_ns = 4500 * MS;
        sc
    });
    for o in &outs {
        books(o);
        if o.seed % 2 == 0 {
            assert_eq!(o.relay.partial_timeouts, 1, "seed {}", o.seed);
            assert!(o.lost >= 1 && o.connects >= 2, "seed {}", o.seed);
            assert!((PARTIAL_READ_MS..=PARTIAL_READ_MS + 5).contains(&o.partial_drop_ms), "seed {}: dropped after {} ms", o.seed, o.partial_drop_ms);
        } else {
            assert_eq!((o.relay.partial_timeouts, o.lost, o.connects), (0, 0, 1), "seed {}: a packet that completes in time is fine", o.seed);
        }
    }
    assert!(
        log.lines().iter().any(|l| l.contains("stalled mid-packet")),
        "the drop must be loud: {:?}",
        log.lines().len()
    );
}

/// (st5) One stall of exactly 10, 49, 50, 51, 200 ms, 2 s, 10 s between two ticks, then the broker
/// RESUMES with a burst of stale ticks (WANT set) and stale releases: the stale numbers are ignored and
/// counted exactly, no backlog replays, no double tick, the tick is handed back once.
#[test]
fn lockstep_soak_st5_stall_then_resume_with_a_stale_burst() {
    let outs = soak("st5 stall, resume with stale burst", |s| {
        let g = stall_dur(s) * MS;
        let mut sc = base_sc("lockstep_soak_st5_stall_then_resume_with_a_stale_burst");
        sc.render = (MS, 5 * MS);
        sc.exact = true;
        for t in [3000 * MS, 3000 * MS + g + 3000 * MS] {
            sc.timeline.push((t, Act::Gap(g)));
            sc.timeline.push((t + g, Act::Resume(24)));
        }
        sc.min_sim_ns = 6000 * MS + 2 * g + 2000 * MS;
        sc
    });
    for o in &outs {
        books(o);
        let g = stall_dur(o.seed);
        let falls = if g >= 50 { 2 } else { 0 };
        assert_eq!(o.handoffs, (1 + falls, falls), "seed {}: gap {g} ms", o.seed);
        assert_eq!(o.relay.silence_fallbacks, if g > 50 { 2 } else { 0 }, "seed {}: gap {g} ms: the relay's rule is 'more than 50 ms'", o.seed);
        assert!(o.stale_pushed >= 48 && o.relay.ticks_stale == o.stale_pushed, "seed {}: stale ticks are counted exactly", o.seed);
        assert_eq!(o.gate.stale, 0, "seed {}", o.seed);
        assert!(o.min_broker_gap_ns >= P60 / 2);
        if g >= 50 {
            assert!(o.probation >= falls, "seed {}: the broker proves itself after a fallback", o.seed);
        }
    }
}

/// (st6) FLAPPING around the boundary: gaps of 49/51, 55, 40/60 and 48..52 ms, for ten seconds. The
/// tick SOURCE does not flap per tick (probation: 1, 2, 4, 8 good gaps); the relay's silence counter is
/// exact; the vblank never stops.
#[test]
fn lockstep_soak_st6_flapping_around_the_boundary() {
    let log = kf_broker::capture_log();
    let outs = soak("st6 flapping", |s| {
        let mut sc = base_sc("lockstep_soak_st6_flapping_around_the_boundary");
        sc.render = (MS, 5 * MS);
        sc.exact = true;
        sc.timeline = vec![(2000 * MS, Act::Mode(Mode::Pattern((s % 4) as usize)))];
        sc.min_sim_ns = 12_000 * MS;
        sc.flips = flips_goal(1500);
        sc
    });
    let mut worst = 0;
    for o in &outs {
        books(o);
        let changes = o.handoffs.0 + o.handoffs.1;
        worst = worst.max(changes);
        assert!(changes <= 14, "seed {}: the tick source changed {changes} times in 10 s of flapping: {:?}", o.seed, o.handoffs);
        assert!(o.relay.silence_fallbacks.abs_diff(o.long_gaps) <= 1, "seed {}: relay silences {} vs gaps over 50 ms {}", o.seed, o.relay.silence_fallbacks, o.long_gaps);
    }
    let lines = log.lines().iter().filter(|l| l.contains("broker ticks:")).count();
    eprintln!(
        "  st6: tick-source changes per run: max {worst} (limit 14) over 10 s; the RELAY logged {lines} 'broker ticks:' lines in {} runs (it has no hysteresis: one line per live/silent change; relay silences {} for {} long gaps)",
        outs.len(),
        total(&outs, |o| o.relay.silence_fallbacks),
        total(&outs, |o| o.long_gaps)
    );
}

/// (st7) SIGSTOP and a broker that never exits: the guest runs on the clock indefinitely (30 s), the
/// stall is visible in a counter and a log line, and a SIGCONT after ten seconds brings the broker back
/// with a burst of stale traffic and one handoff.
#[test]
fn lockstep_soak_st7_sigstop_and_a_hung_broker() {
    let log = kf_broker::capture_log();
    let outs = soak("st7 SIGSTOP / hung", |s| {
        let mut sc = base_sc("lockstep_soak_st7_sigstop_and_a_hung_broker");
        sc.render = (MS, 5 * MS);
        sc.timeline.push((2000 * MS, Act::Hung(true)));
        if s % 2 == 0 {
            // never exits
            sc.min_sim_ns = 30_000 * MS;
        } else {
            sc.timeline.push((12_000 * MS, Act::Hung(false)));
            sc.timeline.push((12_000 * MS, Act::Resume(20)));
            sc.min_sim_ns = 16_000 * MS;
        }
        sc
    });
    for o in &outs {
        books(o);
        assert_eq!(o.relay.silence_fallbacks, 1, "seed {}", o.seed);
        assert!(o.free_ticks > 100, "seed {}: the clock ran the guest", o.seed);
        if o.seed % 2 == 0 {
            assert_eq!(o.handoffs, (1, 1), "seed {}: the clock for ever", o.seed);
        } else {
            assert_eq!(o.handoffs, (2, 1), "seed {}: one handoff back", o.seed);
        }
    }
    assert!(log.lines().iter().any(|l| l.contains("SILENT for 50 ms")), "the stall must be visible in a log line");
}

/// (st8) A stall while a request is waiting for its copy and an IMMEDIATE flip arrives: the flip
/// completes at once (it waits only for the copy in flight; the bound is checked at every step).
#[test]
fn lockstep_soak_st8_stall_during_a_credit_wait_with_a_copy_in_flight() {
    let outs = soak("st8 stall mid credit wait", |s| {
        let mut sc = base_sc("lockstep_soak_st8_stall_during_a_credit_wait_with_a_copy_in_flight");
        sc.gk = GKind::Imm;
        sc.copy = (10 * MS, 25 * MS);
        sc.render = (100_000, 2 * MS);
        for k in 0..6u64 {
            let d = STALLS_MS[((s + k) % 7) as usize].min(2000) * MS;
            let t = 1000 * MS + k * 3000 * MS + (s % 13) * MS;
            match k % 3 {
                0 => {
                    sc.timeline.push((t, Act::Mode(Mode::Silent)));
                    sc.timeline.push((t + d, Act::Mode(Mode::Steady)));
                }
                1 => {
                    sc.timeline.push((t, Act::Hung(true)));
                    sc.timeline.push((t + d, Act::Hung(false)));
                }
                _ => {
                    sc.timeline.push((t, Act::NeverRead(true)));
                    sc.timeline.push((t + d, Act::NeverRead(false)));
                }
            }
        }
        sc.min_sim_ns = 20_000 * MS;
        sc
    });
    for o in &outs {
        books(o);
        assert!(
            o.worst_flip_ns <= 25 * MS + MS / 2 + 2 * MS,
            "seed {}: an immediate flip waited {} ms",
            o.seed,
            o.worst_flip_ns / MS
        );
    }
}

/// G1 in depth: the credit gate under random streams of fresh, stale, duplicate and reordered events
/// (what the worker would see if the relay's own check were gone): the gate's counters equal an
/// independent reference EXACTLY, and a stale event never opens or replaces a credit.
#[test]
fn lockstep_soak_gate_defends_itself_against_stale_events() {
    for seed in seeds() {
        let mut rng = Rng::new(seed ^ 0xfeed);
        let mut g = CreditGate::new();
        let mut last: Option<u32> = None;
        let mut last_req: Option<u32> = None;
        let (mut stale, mut stale_cr, mut granted_ref) = (0u64, 0u64, 0u64);
        let mut seq = 1u32;
        for i in 0..20_000u64 {
            let s = match rng.below(10) {
                0 => seq.wrapping_sub(rng.range(0, 30) as u32).max(1),
                1 => seq,
                _ => {
                    seq = next_seq(seq.wrapping_add(rng.range(0, 3) as u32));
                    seq
                }
            };
            let want = rng.chance(500).then_some(if rng.chance(900) { s } else { s.wrapping_sub(3).max(1) });
            let fresh = last.is_none_or(|l| seq_newer(s, l));
            assert_eq!(g.fresh(s), fresh, "seed {seed} event {i}");
            let emitted = rng.chance(600).then_some(i * 1000);
            let r = g.on_tick(s, 0, want, emitted, Some(i * 1000), 8_000);
            if fresh {
                last = Some(s);
                if let Some(w) = want {
                    if last_req.is_none_or(|l| seq_newer(w, l)) {
                        last_req = Some(w);
                    } else {
                        stale_cr += 1;
                    }
                }
            } else {
                stale += 1;
                stale_cr += u64::from(want.is_some());
            }
            assert_eq!(r, fresh, "seed {seed} event {i}");
            if g.poll(i * 1000 + 9_000).is_some() {
                granted_ref += 1;
            }
        }
        assert_eq!((g.c.stale, g.c.stale_credits, g.c.granted), (stale, stale_cr, granted_ref), "seed {seed}");
    }
}
