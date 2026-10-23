//! ★★ **The lock step over REAL threads and a REAL socket** (`docs/design/V3_DISPLAY_PACING_20261017.md` §11.7).
//!
//! The soak (`src/engine/lockstep_soak.rs`) is deterministic: one thread, an injected clock. This file is
//! its complement — the orderings a scheduler of events cannot express: four threads (a fake BROKER on
//! the far end of a unix socket, the RELAY thread driving `kf_broker::Relay<UnixLink>` over that socket,
//! the display WORKER and the GUEST), real small sleeps, the real kernel socket buffer.
//!
//! REAL: the relay and its framing, `UnixLink` (a non-blocking `AF_UNIX` client), the `FrameRing`, the
//! `TickShare`, `Pacer`/`TickSource`, `CreditGate`, `Mailbox`, `flip_completion_need`. MODELLED: the
//! flip engine (a flip is a latch and a completion queue behind the copy in flight — the real `Engine`
//! is a unit-test fixture of the crate and not reachable here; the soak drives it), the copy (a sleep),
//! the compositor (the fake broker answers requests, presents, releases).
//!
//! ~1000 flips per scenario. A thread that stops making progress is caught by a watchdog (the test fails
//! with the thread's name instead of hanging). Invariants are recorded by the thread that sees them and
//! asserted by the test: every COMMIT answers one open request (once), no frame without a credit while
//! the broker's ticks are live, at most 2 frames in flight, the per-turn budget, no double tick, the
//! vblank never stalls, every flip completes, the immediate flip never waits for a credit.

use kf_broker::conn::READ_BATCH;
use kf_broker::wire::{
    CAP_DMABUF, CAP_FOCUS_EVENTS, CAP_RELEASE, CAP_TICK, CLIENT_TICK, CMD_ATTACH, CMD_CAPS,
    CMD_COMMIT, CMD_NO_CHANGE, CMD_SIZE, CMD_WINDOW, Cmd, EV_FRAME, EV_HELLO, EV_RELEASE,
    FOURCC_XR24, PKT_SIZE, Pkt, TickPkt, seq_newer,
};
use kf_broker::{FrameGeom, FrameRing, Host, Relay, RelayConfig, SlotFds, TickShare, UnixLink};
use kf_disp::engine::HeadMode;
use kf_disp::lockstep::CreditGate;
use kf_disp::pace::{Mailbox, Pacer, flip_completion_need};
use kf_disp::tick::Source;
use kf_linux_raw::SharedRam;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The display's refresh in this file: 5 ms (200 Hz), so a thousand flips take about two seconds.
const P: u64 = 5_000_000;
const MS: u64 = 1_000_000;
const SILENCE: u64 = 50 * MS;

// ------------------------------------------------------------------------------------------------
// shared state: the violations, the heartbeats, the counters

#[derive(Default)]
struct Shared {
    violations: Mutex<Vec<String>>,
    beat: [AtomicU64; 4],
    stop: AtomicBool,
    // facts the test asserts on
    flips: AtomicU64,
    commits: AtomicU64,
    answered: AtomicU64,
    unsolicited: AtomicU64,
    no_changes: AtomicU64,
    ticks_sent: AtomicU64,
    ticks_dropped: AtomicU64,
    max_turn_packets: AtomicU64,
    max_held: AtomicU64,
    worker_fallbacks: AtomicU64,
    connections: AtomicU64,
    relay_lost: AtomicU64,
    relay_dropped: AtomicU64,
    max_gap_ms: AtomicU64,
    max_flip_ms: AtomicU64,
    frames_published: AtomicU64,
}

const T_BROKER: usize = 0;
const T_RELAY: usize = 1;
const T_WORKER: usize = 2;
const T_GUEST: usize = 3;
const NAMES: [&str; 4] = ["broker", "relay", "worker", "guest"];

impl Shared {
    fn bad(&self, why: String) {
        let mut v = self.violations.lock().unwrap();
        if v.len() < 20 {
            v.push(why);
        }
    }
    fn beat(&self, t: usize, base: Instant) {
        self.beat[t].store(base.elapsed().as_micros() as u64 + 1, Ordering::Relaxed);
    }
}

// ------------------------------------------------------------------------------------------------
// the scenario

#[derive(Clone, Copy, Debug)]
enum Act {
    /// Ticks at the refresh.
    Steady,
    /// No ticks.
    Silent,
    /// `burst` ticks written at once every refresh.
    Flood(u32),
    /// The broker stops reading.
    NeverRead(bool),
    /// A partial tick packet (this many bytes), then nothing for a long while.
    Partial(usize),
    /// Close the connection (crash); a new broker accepts later.
    Crash,
    /// A stale burst (old numbers, WANT set).
    StaleBurst(u32),
}

#[derive(Clone)]
struct Scn {
    name: &'static str,
    flips: u64,
    /// `(milliseconds since the start, act)`.
    timeline: Vec<(u64, Act)>,
    /// Every tick asks, whether or not a request is open.
    always_want: bool,
    /// Alternate immediate / vsync flips (two flips a frame).
    alternate: bool,
    /// Copy length range, microseconds.
    copy_us: (u64, u64),
    /// The run lasts at least this long.
    min_ms: u64,
    /// Watchdog: the whole run must end by then.
    limit: Duration,
}

impl Scn {
    fn new(name: &'static str) -> Scn {
        Scn {
            name,
            flips: 1000,
            timeline: Vec::new(),
            always_want: false,
            alternate: false,
            copy_us: (500, 3000),
            min_ms: 0,
            limit: Duration::from_secs(60),
        }
    }
}

// ------------------------------------------------------------------------------------------------
// a tiny deterministic generator for the threads' choices

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo + 1)
    }
}

// ------------------------------------------------------------------------------------------------
// the BROKER thread: the far end of the socket, the compositor, the reference

struct BrokerEnd {
    stream: UnixStream,
    rx: Vec<u8>,
    out: VecDeque<u8>,
    never_read: bool,
    opted: bool,
    tick_seq: u32,
    pkt_seq: u32,
    open: std::collections::BTreeSet<u32>,
    last_label: Option<u32>,
    b_open: Option<(u32, Instant)>,
    unpresented: Option<Instant>,
    last_tick_sent: Instant,
    released: Vec<(Instant, u64)>,
    reported_released: Vec<u64>,
}

const OUT_CAP: usize = 2048 * PKT_SIZE;

impl BrokerEnd {
    fn new(stream: UnixStream, never_read: bool) -> BrokerEnd {
        BrokerEnd {
            stream,
            rx: Vec::new(),
            out: VecDeque::new(),
            never_read,
            opted: false,
            tick_seq: 0,
            pkt_seq: 0,
            open: std::collections::BTreeSet::new(),
            last_label: None,
            b_open: None,
            unpresented: None,
            last_tick_sent: Instant::now(),
            released: Vec::new(),
            reported_released: Vec::new(),
        }
    }

    fn hello(&mut self, sh: &Shared) {
        self.push(
            sh,
            Pkt {
                ty: EV_HELLO,
                w0: 2,
                w1: CAP_DMABUF | CAP_FOCUS_EVENTS | CAP_RELEASE | CAP_TICK,
                ..Pkt::default()
            },
        );
    }

    /// Read what the relay sent and judge every command (`on_cmd`). `false`: the connection is gone.
    fn read_cmds(&mut self, sh: &Shared, ring: &FrameRing, now: Instant) -> bool {
        let mut buf = [0u8; 4096];
        loop {
            match self.stream.read(&mut buf) {
                Ok(0) => return false,
                Ok(n) => self.rx.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return false,
            }
        }
        while self.rx.len() >= CMD_SIZE {
            let rec: [u8; CMD_SIZE] = self.rx[..CMD_SIZE].try_into().unwrap();
            self.rx.drain(..CMD_SIZE);
            let Some(cmd) = Cmd::decode(&rec) else {
                sh.bad("the relay sent a command with reserved1 != 0".into());
                continue;
            };
            on_cmd(self, sh, ring, cmd, now);
        }
        true
    }

    fn push(&mut self, sh: &Shared, mut p: Pkt) -> bool {
        self.pkt_seq = self.pkt_seq.wrapping_add(1);
        p.seq = self.pkt_seq;
        if self.out.len() + PKT_SIZE > OUT_CAP {
            sh.ticks_dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        self.out.extend(p.encode());
        true
    }

    fn flush(&mut self) {
        loop {
            let (a, _) = self.out.as_slices();
            if a.is_empty() {
                return;
            }
            match self.stream.write(a) {
                Ok(0) => return,
                Ok(n) => {
                    self.out.drain(..n);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return,
            }
        }
    }
}

fn broker_thread(
    listener: UnixListener,
    sh: Arc<Shared>,
    scn: Scn,
    ring: Arc<FrameRing>,
    base: Instant,
) {
    listener.set_nonblocking(true).unwrap();
    let mut rng = Rng(0x1234_5678_9abc_def1);
    // the timeline runs once, across connections
    let mut tl: VecDeque<(u64, Act)> = scn.timeline.iter().copied().collect();
    let mut mode = Act::Steady;
    let mut never_read = false;
    let mut partial_until: Option<Instant> = None;
    while !sh.stop.load(Ordering::Relaxed) {
        let stream = match listener.accept() {
            Ok((s, _)) => s,
            Err(_) => {
                sh.beat(T_BROKER, base);
                std::thread::sleep(Duration::from_millis(1));
                continue;
            }
        };
        stream.set_nonblocking(true).unwrap();
        sh.connections.fetch_add(1, Ordering::Relaxed);
        let mut b = BrokerEnd::new(stream, never_read);
        b.hello(&sh);
        let mut next_tick = Instant::now();
        'conn: while !sh.stop.load(Ordering::Relaxed) {
            sh.beat(T_BROKER, base);
            let now = Instant::now();
            // the timeline
            while tl
                .front()
                .is_some_and(|(t, _)| base.elapsed().as_millis() as u64 >= *t)
            {
                let (_, a) = tl.pop_front().unwrap();
                match a {
                    Act::Steady | Act::Silent | Act::Flood(_) => mode = a,
                    Act::NeverRead(n) => {
                        never_read = n;
                        b.never_read = n;
                    }
                    Act::Partial(n) => {
                        let t = TickPkt {
                            tick_seq: b.tick_seq.wrapping_add(1).max(1),
                            refresh_ns: P as u32,
                            want: true,
                            clock_only: false,
                            present_ns: 0,
                        };
                        let bytes = Pkt::tick_pkt(0, 0, &t).encode();
                        let _ = b.stream.write(&bytes[..n.clamp(1, PKT_SIZE - 1)]);
                        partial_until = Some(now + Duration::from_millis(900));
                    }
                    Act::Crash => break 'conn,
                    Act::StaleBurst(n) => {
                        for i in 0..n {
                            let t = TickPkt {
                                tick_seq: b.tick_seq.wrapping_sub(1 + i).max(1),
                                refresh_ns: P as u32,
                                want: true,
                                clock_only: false,
                                present_ns: 0,
                            };
                            b.pkt_seq = b.pkt_seq.wrapping_add(1);
                            if b.out.len() + PKT_SIZE <= OUT_CAP {
                                b.out.extend(Pkt::tick_pkt(b.pkt_seq, 0, &t).encode());
                            }
                        }
                    }
                }
            }
            if partial_until.is_some_and(|t| now < t) {
                // a stalled broker: it writes nothing and reads nothing until the relay gives up
                std::thread::sleep(Duration::from_millis(2));
                let mut buf = [0u8; 8];
                if matches!(b.stream.read(&mut buf), Ok(0)) {
                    partial_until = None;
                    break 'conn;
                }
                continue;
            }
            partial_until = None;
            // read what the relay sent
            if !b.never_read && !b.read_cmds(&sh, &ring, now) {
                break 'conn;
            }
            // compositor: presentation and releases (a broker that stopped READING still releases)
            if b.never_read {
                release_old(&mut b, &ring, now);
            }
            if b.unpresented.is_some_and(|t| now >= t) {
                b.unpresented = None;
            }
            let due: Vec<u64> = b
                .released
                .iter()
                .filter(|(t, _)| *t <= now)
                .map(|(_, id)| *id)
                .collect();
            b.released.retain(|(t, _)| *t > now);
            for id in due {
                b.push(
                    &sh,
                    Pkt {
                        ty: EV_RELEASE,
                        w0: id as u32,
                        w1: (id >> 32) as u32,
                        ..Pkt::default()
                    },
                );
            }
            // ticks
            if b.opted && now >= next_tick {
                let (n, interval) = match mode {
                    Act::Steady => (1u32, Duration::from_nanos(P)),
                    Act::Flood(burst) => (burst, Duration::from_nanos(P)),
                    _ => (0, Duration::from_millis(1)),
                };
                for _ in 0..n {
                    emit_tick(&mut b, &sh, &scn, now, &mut rng);
                }
                next_tick = now + interval;
            }
            // noise of other kinds now and then
            if rng.range(0, 40) == 0 {
                b.push(
                    &sh,
                    Pkt {
                        ty: EV_FRAME,
                        ..Pkt::default()
                    },
                );
                b.push(
                    &sh,
                    Pkt {
                        ty: 900 + rng.range(0, 50) as u16,
                        ..Pkt::default()
                    },
                );
            }
            b.flush();
            std::thread::sleep(Duration::from_micros(150));
        }
        drop(b);
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn emit_tick(b: &mut BrokerEnd, sh: &Shared, scn: &Scn, now: Instant, rng: &mut Rng) {
    if b.unpresented
        .is_some_and(|t| now >= t + Duration::from_millis(100))
    {
        b.unpresented = None;
    }
    let timed_out = b
        .b_open
        .is_some_and(|(_, at)| now >= at + Duration::from_millis(100));
    let want = scn.always_want || ((b.b_open.is_none() || timed_out) && b.unpresented.is_none());
    b.tick_seq = match b.tick_seq.wrapping_add(1) {
        0 => 1,
        n => n,
    };
    let t = TickPkt {
        tick_seq: b.tick_seq,
        refresh_ns: P as u32,
        want,
        clock_only: false,
        present_ns: now.duration_since(b.last_tick_sent).as_nanos() as u64,
    };
    if want {
        b.b_open = Some((b.tick_seq, now));
        b.open.insert(b.tick_seq);
        if b.open.len() > 100_000 {
            let first = *b.open.iter().next().unwrap();
            b.open.remove(&first);
        }
    }
    b.last_tick_sent = now;
    b.pkt_seq = b.pkt_seq.wrapping_add(1);
    if b.out.len() + PKT_SIZE > OUT_CAP {
        sh.ticks_dropped.fetch_add(1, Ordering::Relaxed);
        return;
    }
    b.out.extend(Pkt::tick_pkt(b.pkt_seq, 0, &t).encode());
    sh.ticks_sent.fetch_add(1, Ordering::Relaxed);
    let _ = rng;
}

/// The compositor releases every held frame but the newest, a little later.
fn release_old(b: &mut BrokerEnd, ring: &FrameRing, now: Instant) {
    let held = ring.held_mask();
    let mut newest: Option<(u64, usize)> = None;
    for j in 0..ring.slots() {
        if held & (1 << j) != 0 {
            let s = ring.geometry(j).serial;
            if newest.is_none_or(|(ns, _)| s > ns) {
                newest = Some((s, j));
            }
        }
    }
    for j in 0..ring.slots() {
        if held & (1 << j) != 0 && Some(j) != newest.map(|n| n.1) {
            if let Some(f) = ring.fds(j) {
                b.released
                    .push((now + Duration::from_millis(3), f.memfd_id()));
            }
        }
    }
}

fn on_cmd(b: &mut BrokerEnd, sh: &Shared, ring: &FrameRing, cmd: Cmd, now: Instant) {
    match cmd.ty {
        CMD_WINDOW | CMD_ATTACH => {}
        CMD_CAPS => {
            b.opted = cmd.width & CLIENT_TICK != 0;
            if b.opted && cmd.height != 60 {
                sh.bad(format!(
                    "CAPS opted in with ceiling {}, expected the VMM's 60",
                    cmd.height
                ));
            }
        }
        CMD_COMMIT => {
            sh.commits.fetch_add(1, Ordering::Relaxed);
            let label = cmd.seq;
            if b.opted {
                if label == 0 {
                    // unsolicited: allowed only while the ticks have been silent (the relay's rule is
                    // 50 ms on ITS clock; the broker's send time is earlier, so 40 ms here)
                    if now.duration_since(b.last_tick_sent) < Duration::from_millis(40) {
                        sh.bad(format!(
                            "a frame without a credit (COMMIT.seq 0) {} ms after the last tick",
                            now.duration_since(b.last_tick_sent).as_millis()
                        ));
                    }
                    sh.unsolicited.fetch_add(1, Ordering::Relaxed);
                } else {
                    if !b.open.contains(&label) {
                        sh.bad(format!(
                            "COMMIT.seq {label} is not an open request ({:?})",
                            b.open
                        ));
                    }
                    if b.last_label.is_some_and(|l| !seq_newer(label, l)) {
                        sh.bad(format!(
                            "request {label} answered again / out of order (last {:?})",
                            b.last_label
                        ));
                    }
                    b.last_label = Some(label);
                    b.open.remove(&label);
                    if b.b_open.map(|(s, _)| s) == Some(label) {
                        b.b_open = None;
                    }
                    sh.answered.fetch_add(1, Ordering::Relaxed);
                }
            }
            b.unpresented = Some(now + Duration::from_micros(1000 + (b.pkt_seq as u64 % 4) * 1000));
            release_old(b, ring, now);
            let _ = &b.reported_released;
        }
        CMD_NO_CHANGE => {
            sh.no_changes.fetch_add(1, Ordering::Relaxed);
            let label = cmd.width;
            if !b.open.contains(&label) {
                sh.bad(format!(
                    "NO_CHANGE({label}) answers no open request ({:?})",
                    b.open
                ));
            }
            if b.last_label.is_some_and(|l| !seq_newer(label, l)) {
                sh.bad(format!("request {label} answered twice (NO_CHANGE)"));
            }
            b.last_label = Some(label);
            b.open.remove(&label);
            if b.b_open.map(|(s, _)| s) == Some(label) {
                b.b_open = None;
            }
        }
        other => sh.bad(format!("the relay sent an unexpected command type {other}")),
    }
}

// ------------------------------------------------------------------------------------------------
// the RELAY thread

struct H {
    timer: Option<u64>,
}
impl Host for H {
    fn watch(&mut self, _fd: i32, _r: bool, _w: bool) {}
    fn timer(&mut self, d: Option<u64>) {
        self.timer = d;
    }
}

fn relay_thread(
    path: std::path::PathBuf,
    sh: Arc<Shared>,
    ring: Arc<FrameRing>,
    share: Arc<TickShare>,
    frame_wake: Arc<AtomicBool>,
    base: Instant,
) {
    let mut relay = Relay::new(
        RelayConfig {
            path,
            extra_uid: None,
        },
        ring.clone(),
        UnixLink,
    )
    .with_ticks(share);
    let mut host = H { timer: None };
    let ms = |base: Instant| base.elapsed().as_millis() as u64;
    relay.start(ms(base), &mut host);
    let mut was_conn = false;
    let mut out = Vec::new();
    while !sh.stop.load(Ordering::Relaxed) {
        sh.beat(T_RELAY, base);
        let now = ms(base);
        if host.timer.is_some_and(|t| t <= now) {
            relay.on_timer(now, &mut host);
        }
        if frame_wake.swap(false, Ordering::AcqRel) {
            relay.on_frame(now, &mut host);
        }
        if relay.connected() {
            out.clear();
            let p0 = relay.counters().packets;
            relay.on_socket(now, true, true, &mut host, &mut out, READ_BATCH);
            let d = relay.counters().packets - p0;
            sh.max_turn_packets.fetch_max(d, Ordering::Relaxed);
            if d > READ_BATCH as u64 {
                sh.bad(format!(
                    "a relay turn processed {d} packets (budget {READ_BATCH})"
                ));
            }
        }
        let c = relay.connected();
        if was_conn && !c {
            sh.relay_lost.fetch_add(1, Ordering::Relaxed);
        }
        was_conn = c;
        let c = relay.counters();
        sh.relay_dropped
            .fetch_max(c.dropped + c.uncommitted, Ordering::Relaxed);
        let held = u64::from(ring.held_mask().count_ones());
        sh.max_held.fetch_max(held, Ordering::Relaxed);
        if held > 2 {
            sh.bad(format!("{held} frames in flight"));
        }
        std::thread::sleep(Duration::from_micros(100));
    }
    relay.stop(&mut host);
}

// ------------------------------------------------------------------------------------------------
// the WORKER thread: kf-qemu display.rs steps 4 and 6, with a model of the flips

enum ToWorker {
    Flip { id: u64, imm: bool },
}

/// The copy into `s` is done: the frame goes to the ring (`display.rs` step 6).
fn publish_frame(ring: &FrameRing, s: usize, serial: u64) {
    ring.describe(
        s,
        FrameGeom {
            width: 64,
            height: 64,
            stride: 256,
            fourcc: FOURCC_XR24,
            serial,
            cursor: false,
        },
    );
    ring.publish(s);
    let _ = ring.take_console();
}

#[allow(clippy::too_many_lines)]
fn worker_thread(
    sh: Arc<Shared>,
    scn: Scn,
    ring: Arc<FrameRing>,
    share: Arc<TickShare>,
    frame_wake: Arc<AtomicBool>,
    rx: mpsc::Receiver<ToWorker>,
    done: mpsc::Sender<u64>,
    base: Instant,
) {
    let ns = |t: Instant| t.duration_since(base).as_nanos() as u64;
    let mut pacer = Pacer::new(200);
    pacer.on_heads(
        &[HeadMode {
            head: 0,
            period_ns: P,
            raster: (2200, 1125),
        }],
        ns(Instant::now()),
    );
    let mut gate = CreditGate::new();
    let mut mailbox = Mailbox::default();
    let mut tick_seen = 0u64;
    let mut rng = Rng(0xfeed_beef_1234_5678);
    // flips: (needs copy, flip id); latched but not yet completed vsync flips wait for a tick
    let mut queue: VecDeque<(u64, u64)> = VecDeque::new();
    let mut vsync_wait: VecDeque<u64> = VecDeque::new();
    let (mut started, mut finished) = (0u64, 0u64);
    let mut inflight: Option<(u64, Instant, Option<usize>)> = None;
    let mut last_tick: Option<(u64, Source)> = None;
    let mut last_vseq = 0u64;
    let mut serial = 0u64;
    let mut copies = 0u64;
    let mut credits_seen = 0u64;
    let mut free_ticks = 0u64;
    let mut handoffs_to_free = 0u64;
    let mut polled: Option<u32> = None;
    while !sh.stop.load(Ordering::Relaxed) {
        sh.beat(T_WORKER, base);
        let now_i = Instant::now();
        let now = ns(now_i);
        // flips from the guest (the statement the drainer hands over)
        while let Ok(ToWorker::Flip { id, imm }) = rx.try_recv() {
            if imm {
                // an immediate flip latches at once and completes behind the copy in flight only
                mailbox.latch();
                let need = flip_completion_need(0, inflight.map(|f| f.0));
                queue.push_back((need, id));
            } else {
                vsync_wait.push_back(id);
            }
        }
        // 4. ticks: the free clock's, and the broker's through the share
        let mut got = pacer.due(now);
        if let Some(snap) = share.snapshot(tick_seen) {
            let coalesced = snap.count.saturating_sub(tick_seen + 1);
            tick_seen = snap.count;
            let want = share.take_want();
            let t = if gate.fresh(snap.tick_seq) {
                pacer.broker_tick(0, now)
            } else {
                None
            };
            let (last, guard) = pacer
                .source(0)
                .map_or((None, 0), |s| (s.last_tick_ns(), s.guard_ns()));
            gate.on_tick(snap.tick_seq, coalesced, want, t.map(|_| now), last, guard);
            got.extend(t);
        }
        if let Some(req) = gate.poll(now) {
            if polled != Some(req) {
                polled = Some(req);
                credits_seen += 1;
            }
            mailbox.grant();
        }
        for (_, ho, _) in pacer.take_handoffs() {
            if ho == kf_disp::tick::Handoff::ToFree {
                handoffs_to_free += 1;
                sh.worker_fallbacks.fetch_add(1, Ordering::Relaxed);
            }
        }
        for t in got {
            if t.seq != last_vseq + 1 {
                sh.bad(format!("the vblank counter went {last_vseq} -> {}", t.seq));
            }
            last_vseq = t.seq;
            if let Some((l, _)) = last_tick {
                let gap = now - l;
                sh.max_gap_ms.fetch_max(gap / MS, Ordering::Relaxed);
                if t.from == Source::Broker {
                    let guard = pacer.source(0).map_or(P / 2, |s| s.guard_ns());
                    if gap + 300_000 < guard {
                        sh.bad(format!(
                            "a double tick: {gap} ns after the previous (guard {guard})"
                        ));
                    }
                }
            }
            last_tick = Some((now, t.from));
            if t.from == Source::Free {
                mailbox.tick();
                free_ticks += 1;
            }
            // a vsync flip waiting for its tick latches (and, behind the copy in flight, completes)
            if let Some(id) = vsync_wait.pop_front() {
                mailbox.latch();
                let need = flip_completion_need(0, inflight.map(|f| f.0));
                queue.push_back((need, id));
            }
        }
        if let Some((l, _)) = last_tick
            && now - l > SILENCE + 2 * P + 60 * MS
        {
            sh.bad(format!("the vblank stalled for {} ms", (now - l) / MS));
            last_tick = Some((now, Source::Free));
        }
        // 6. the copy completes and the frame goes to the ring; the next one starts on a credit
        if let Some((n, end, slot)) = inflight
            && now_i >= end
        {
            inflight = None;
            finished = n;
            if let Some(s) = slot {
                serial += 1;
                publish_frame(&ring, s, serial);
                sh.frames_published.fetch_add(1, Ordering::Relaxed);
                frame_wake.store(true, Ordering::Release);
            }
        }
        if mailbox.due(inflight.is_some()) {
            let slot = ring.fill_target(None);
            match slot {
                Some(s) if ring.held_mask() & (1 << s) == 0 && ring.broker_ready() != Some(s) => {}
                other => sh.bad(format!(
                    "the fill target {other:?} is held or ready (held {:#x})",
                    ring.held_mask()
                )),
            }
            started += 1;
            copies += 1;
            mailbox.copied();
            let us = rng.range(scn.copy_us.0, scn.copy_us.1);
            inflight = Some((started, now_i + Duration::from_micros(us), slot));
        }
        if copies > free_ticks + credits_seen + 1 {
            sh.bad(format!(
                "{copies} copies but {free_ticks} free ticks and {credits_seen} credits"
            ));
            copies = 0;
        }
        while queue.front().is_some_and(|(need, _)| *need <= finished) {
            let (_, id) = queue.pop_front().unwrap();
            let _ = done.send(id);
        }
        let _ = handoffs_to_free;
        // sleep until the next thing is due (at most 300 µs: the relay and the guest are other threads)
        let mut wait = Duration::from_micros(300);
        if let Some(t) = pacer.next_ns() {
            wait = wait.min(Duration::from_nanos(t.saturating_sub(ns(Instant::now()))));
        }
        if let Some((_, end, _)) = inflight {
            wait = wait.min(end.saturating_duration_since(Instant::now()));
        }
        std::thread::sleep(wait.max(Duration::from_micros(20)));
    }
}

// ------------------------------------------------------------------------------------------------
// the GUEST thread

fn guest_thread(
    sh: Arc<Shared>,
    scn: Scn,
    tx: mpsc::Sender<ToWorker>,
    done: mpsc::Receiver<u64>,
    base: Instant,
) {
    let mut rng = Rng(0x0bad_cafe_dead_beef);
    for id in 0..scn.flips {
        sh.beat(T_GUEST, base);
        std::thread::sleep(Duration::from_micros(rng.range(100, 1500)));
        let imm = !scn.alternate || id % 2 == 0;
        let t0 = Instant::now();
        if tx.send(ToWorker::Flip { id, imm }).is_err() {
            return;
        }
        loop {
            match done.recv_timeout(Duration::from_millis(20)) {
                Ok(d) if d == id => break,
                Ok(d) => {
                    sh.bad(format!(
                        "flip {d} completed while flip {id} was outstanding"
                    ));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    sh.beat(T_GUEST, base);
                    if t0.elapsed() > Duration::from_millis(400) {
                        sh.bad(format!(
                            "flip {id} ({}) did not complete in 400 ms",
                            if imm { "immediate" } else { "vsync" }
                        ));
                        return;
                    }
                }
                Err(_) => return,
            }
        }
        let ms = t0.elapsed().as_millis() as u64;
        sh.max_flip_ms.fetch_max(ms, Ordering::Relaxed);
        if imm && ms > 60 {
            sh.bad(format!(
                "an immediate flip took {ms} ms (it waits for the copy in flight and nothing else)"
            ));
        }
        sh.flips.fetch_add(1, Ordering::Relaxed);
    }
}

// ------------------------------------------------------------------------------------------------
// the run

struct Result {
    sh: Arc<Shared>,
    elapsed: Duration,
}

fn frame_ring() -> Arc<FrameRing> {
    let ring = Arc::new(FrameRing::new(kf_broker::slots::BROKER_SLOTS, true));
    for j in 0..ring.slots() {
        let mem = SharedRam::create_named(c"kfb-threads-frame", 64 * 1024).unwrap();
        ring.install(j, SlotFds::new(mem, None).unwrap()).unwrap();
    }
    ring
}

fn run(scn: Scn) -> Result {
    let base = Instant::now();
    let dir = std::env::temp_dir().join(format!(
        "kf-lockstep-threads-{}-{}",
        std::process::id(),
        scn.name
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("display.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let sh = Arc::new(Shared::default());
    let ring = frame_ring();
    let share = Arc::new(TickShare::new());
    share.set_cap_hz(60);
    let frame_wake = Arc::new(AtomicBool::new(false));
    let (to_w, from_g) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let mut handles = Vec::new();
    {
        let (sh, scn, ring) = (sh.clone(), scn.clone(), ring.clone());
        handles.push(std::thread::spawn(move || {
            broker_thread(listener, sh, scn, ring, base)
        }));
    }
    {
        let (sh, ring, share, fw, path) = (
            sh.clone(),
            ring.clone(),
            share.clone(),
            frame_wake.clone(),
            path.clone(),
        );
        handles.push(std::thread::spawn(move || {
            relay_thread(path, sh, ring, share, fw, base)
        }));
    }
    {
        let (sh, scn, ring, share, fw) = (
            sh.clone(),
            scn.clone(),
            ring.clone(),
            share.clone(),
            frame_wake.clone(),
        );
        handles.push(std::thread::spawn(move || {
            worker_thread(sh, scn, ring, share, fw, from_g, done_tx, base)
        }));
    }
    let guest = {
        let (sh, scn) = (sh.clone(), scn.clone());
        std::thread::spawn(move || guest_thread(sh, scn, to_w, done_rx, base))
    };
    // the watchdog: the guest finishes its flips (and the run lasts its minimum), or a thread is named
    let deadline = base + scn.limit;
    loop {
        std::thread::sleep(Duration::from_millis(20));
        let done_flips = sh.flips.load(Ordering::Relaxed) >= scn.flips;
        if done_flips && base.elapsed() >= Duration::from_millis(scn.min_ms) {
            break;
        }
        let now_us = base.elapsed().as_micros() as u64;
        for (i, b) in sh.beat.iter().enumerate() {
            let last = b.load(Ordering::Relaxed);
            if i != T_GUEST || !done_flips {
                if last != 0 && now_us.saturating_sub(last) > 1_500_000 {
                    sh.bad(format!(
                        "the {} thread made no progress for {} ms: it is BLOCKED",
                        NAMES[i],
                        (now_us - last) / 1000
                    ));
                    sh.stop.store(true, Ordering::Relaxed);
                }
            }
        }
        if sh.stop.load(Ordering::Relaxed) || Instant::now() > deadline {
            if Instant::now() > deadline {
                sh.bad(format!(
                    "the run did not finish in {:?}: {} of {} flips",
                    scn.limit,
                    sh.flips.load(Ordering::Relaxed),
                    scn.flips
                ));
            }
            break;
        }
        if guest.is_finished() && !done_flips {
            break;
        }
    }
    let elapsed = base.elapsed();
    sh.stop.store(true, Ordering::Relaxed);
    let _ = guest.join();
    for h in handles {
        // a thread that is blocked for good cannot be joined: give it a moment and walk away
        let t0 = Instant::now();
        while !h.is_finished() && t0.elapsed() < Duration::from_millis(800) {
            std::thread::sleep(Duration::from_millis(5));
        }
        if h.is_finished() {
            let _ = h.join();
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    Result { sh, elapsed }
}

fn check(r: &Result, scn: &Scn) {
    let v = r.sh.violations.lock().unwrap();
    assert!(
        v.is_empty(),
        "{}: {} violation(s): {:#?}",
        scn.name,
        v.len(),
        *v
    );
    assert!(
        r.sh.flips.load(Ordering::Relaxed) >= scn.flips,
        "{}: {} of {} flips",
        scn.name,
        r.sh.flips.load(Ordering::Relaxed),
        scn.flips
    );
    assert!(r.sh.max_turn_packets.load(Ordering::Relaxed) <= READ_BATCH as u64);
    assert!(r.sh.max_held.load(Ordering::Relaxed) <= 2);
}

fn report(r: &Result, scn: &Scn) {
    let s = &r.sh;
    eprintln!(
        "lockstep_threads {}: {} flips in {} ms; ticks sent {} (dropped by the broker's full buffer {}), commits {} (answered {}, unsolicited {}, no_change {}), frames published {}, worker fallbacks {}, relay drops {}, connections {}, max relay turn {} packets, max held {}, max vblank gap {} ms, max flip {} ms",
        scn.name,
        s.flips.load(Ordering::Relaxed),
        r.elapsed.as_millis(),
        s.ticks_sent.load(Ordering::Relaxed),
        s.ticks_dropped.load(Ordering::Relaxed),
        s.commits.load(Ordering::Relaxed),
        s.answered.load(Ordering::Relaxed),
        s.unsolicited.load(Ordering::Relaxed),
        s.no_changes.load(Ordering::Relaxed),
        s.frames_published.load(Ordering::Relaxed),
        s.worker_fallbacks.load(Ordering::Relaxed),
        s.relay_lost.load(Ordering::Relaxed),
        s.connections.load(Ordering::Relaxed),
        s.max_turn_packets.load(Ordering::Relaxed),
        s.max_held.load(Ordering::Relaxed),
        s.max_gap_ms.load(Ordering::Relaxed),
        s.max_flip_ms.load(Ordering::Relaxed),
    );
}

// ------------------------------------------------------------------------------------------------
// the scenarios

/// A correct broker at the refresh: ~1000 immediate flips, one frame per request, nothing unsolicited.
#[test]
fn lockstep_threads_steady() {
    let scn = Scn::new("lockstep_threads_steady");
    let r = run(scn.clone());
    check(&r, &scn);
    report(&r, &scn);
    let s = &r.sh;
    assert!(
        s.answered.load(Ordering::Relaxed) > 50,
        "frames must flow on requests"
    );
    assert_eq!(s.unsolicited.load(Ordering::Relaxed), 0);
    assert_eq!(s.worker_fallbacks.load(Ordering::Relaxed), 0);
}

/// Two flips per frame (immediate + vsync): the vsync flip waits for a tick, the immediate one never.
#[test]
fn lockstep_threads_two_flips_per_frame() {
    let mut scn = Scn::new("lockstep_threads_two_flips_per_frame");
    scn.alternate = true;
    let r = run(scn.clone());
    check(&r, &scn);
    report(&r, &scn);
}

/// Tick floods (a burst of 100 every refresh, a WANT on every tick) in windows, with other message
/// kinds interleaved: the per-turn budget binds, the worker sees the latest only, one request is open.
#[test]
fn lockstep_threads_flood() {
    let mut scn = Scn::new("lockstep_threads_flood");
    scn.always_want = true;
    scn.timeline = vec![
        (300, Act::Flood(100)),
        (500, Act::Steady),
        (800, Act::Flood(300)),
        (1000, Act::Steady),
        (1300, Act::Flood(1000)),
        (1500, Act::Steady),
    ];
    scn.min_ms = 2000;
    let r = run(scn.clone());
    check(&r, &scn);
    report(&r, &scn);
    assert!(
        r.sh.max_turn_packets.load(Ordering::Relaxed) >= 60,
        "the budget must have bound"
    );
}

/// The broker goes silent for 200 ms twice: the clock takes over, frames keep flowing on the legacy
/// credit, the broker gets the tick back.
#[test]
fn lockstep_threads_silence_and_return() {
    let mut scn = Scn::new("lockstep_threads_silence_and_return");
    scn.timeline = vec![
        (300, Act::Silent),
        (500, Act::Steady),
        (900, Act::Silent),
        (1100, Act::StaleBurst(20)),
        (1100, Act::Steady),
    ];
    scn.min_ms = 2000;
    let r = run(scn.clone());
    check(&r, &scn);
    report(&r, &scn);
    assert!(r.sh.worker_fallbacks.load(Ordering::Relaxed) >= 2);
    assert!(
        r.sh.unsolicited.load(Ordering::Relaxed) >= 1,
        "frames must flow in the fallback"
    );
}

/// The broker stops READING while it still ticks: the relay's sends meet a full socket, frames are
/// dropped and nothing blocks (the relay thread keeps its heartbeat), the guest does not notice.
#[test]
fn lockstep_threads_broker_never_reads() {
    let mut scn = Scn::new("lockstep_threads_broker_never_reads");
    // a request on every tick keeps the relay sending, so the kernel socket buffer really fills
    scn.always_want = true;
    scn.timeline = vec![(300, Act::NeverRead(true))];
    scn.min_ms = 4000;
    let r = run(scn.clone());
    check(&r, &scn);
    report(&r, &scn);
    assert!(
        r.sh.relay_dropped.load(Ordering::Relaxed) > 0,
        "the socket buffer must really have filled (a send met EAGAIN and the frame was dropped and counted)"
    );
}

/// A partial packet and then nothing: the relay drops the connection at its read deadline (500 ms),
/// reconnects, and the run goes on.
#[test]
fn lockstep_threads_partial_packet_then_nothing() {
    let mut scn = Scn::new("lockstep_threads_partial_packet_then_nothing");
    scn.timeline = vec![(400, Act::Partial(10))];
    scn.min_ms = 3500;
    let r = run(scn.clone());
    check(&r, &scn);
    report(&r, &scn);
    assert!(
        r.sh.relay_lost.load(Ordering::Relaxed) >= 1,
        "the stalled connection must be dropped"
    );
    assert!(
        r.sh.connections.load(Ordering::Relaxed) >= 2,
        "and the relay must reconnect"
    );
}

/// The broker crashes and a new one accepts: the relay reconnects, the new connection's numbering
/// starts over, the guest never notices.
#[test]
fn lockstep_threads_crash_and_reconnect() {
    let mut scn = Scn::new("lockstep_threads_crash_and_reconnect");
    scn.timeline = vec![(500, Act::Crash), (1500, Act::Crash)];
    scn.min_ms = 3500;
    let r = run(scn.clone());
    check(&r, &scn);
    report(&r, &scn);
    assert!(r.sh.connections.load(Ordering::Relaxed) >= 3);
}

// ------------------------------------------------------------------------------------------------
// ★ M3 (review of candidate 6d, 2026-10-23): the crossing at the resume, replayed on ONE thread

/// One relay entry (what `relay_thread` calls).
#[derive(Clone, Copy)]
enum Entry {
    Timer,
    Frame,
    Socket,
}

/// The REAL relay and the broker end above over a real socket, driven by ONE thread on an injected
/// clock (the relay's milliseconds; the broker's `Instant`s are `base + ms`), judged by the same
/// oracle as the threaded runs (`on_cmd`). An ordering the scheduler hits now and then in
/// `lockstep_threads_silence_and_return` is written down here and happens every time.
struct Bench {
    sh: Arc<Shared>,
    scn: Scn,
    ring: Arc<FrameRing>,
    relay: Relay<UnixLink>,
    host: H,
    b: BrokerEnd,
    rng: Rng,
    base: Instant,
    serial: u64,
    dir: std::path::PathBuf,
}

impl Drop for Bench {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Bench {
    /// Connected, HELLO answered, the relay opted in to ticks (t = 1 ms).
    fn new(name: &'static str) -> Bench {
        let base = Instant::now();
        let dir = std::env::temp_dir().join(format!(
            "kf-lockstep-bench-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("display.sock");
        let listener = UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let sh = Arc::new(Shared::default());
        let ring = frame_ring();
        let share = Arc::new(TickShare::new());
        share.set_cap_hz(60);
        let cfg = RelayConfig {
            path,
            extra_uid: None,
        };
        let mut relay = Relay::new(cfg, ring.clone(), UnixLink).with_ticks(share);
        let mut host = H { timer: None };
        relay.start(0, &mut host);
        relay.on_timer(0, &mut host);
        let (stream, _) = listener
            .accept()
            .expect("the relay connects at its first attempt");
        stream.set_nonblocking(true).unwrap();
        let mut b = BrokerEnd::new(stream, false);
        b.hello(&sh);
        let mut x = Bench {
            sh,
            scn: Scn::new(name),
            ring,
            relay,
            host,
            b,
            rng: Rng(0x1234_5678_9abc_def1),
            base,
            serial: 0,
            dir,
        };
        x.b.flush();
        x.relay(1, Entry::Socket);
        x.broker(1);
        assert!(x.b.opted, "the relay opted in to the broker's ticks");
        x
    }

    fn at(&self, ms: u64) -> Instant {
        self.base + Duration::from_millis(ms)
    }

    fn relay(&mut self, ms: u64, e: Entry) {
        let mut out = Vec::new();
        match e {
            Entry::Timer => self.relay.on_timer(ms, &mut self.host),
            Entry::Frame => self.relay.on_frame(ms, &mut self.host),
            Entry::Socket => {
                self.relay
                    .on_socket(ms, true, true, &mut self.host, &mut out, READ_BATCH);
            }
        }
    }

    /// The broker's turn at `ms`: read and judge what the relay sent, present, flush.
    fn broker(&mut self, ms: u64) {
        let now = self.at(ms);
        assert!(
            self.b.read_cmds(&self.sh, &self.ring, now),
            "the relay closed the connection"
        );
        if self.b.unpresented.is_some_and(|t| now >= t) {
            self.b.unpresented = None;
        }
        self.b.flush();
    }

    fn tick(&mut self, ms: u64) {
        let now = self.at(ms);
        emit_tick(&mut self.b, &self.sh, &self.scn, now, &mut self.rng);
    }

    fn event(&mut self, ty: u16) {
        self.b.push(
            &self.sh,
            Pkt {
                ty,
                ..Pkt::default()
            },
        );
    }

    /// The worker's copy is done: a frame is ready for the relay.
    fn frame(&mut self) {
        let s = self.ring.fill_target(None).expect("a free frame slot");
        self.serial += 1;
        publish_frame(&self.ring, s, self.serial);
    }

    fn count(&self, c: &AtomicU64) -> u64 {
        c.load(Ordering::Relaxed)
    }
}

/// Up to the crossing: live (a tick with WANT, read, answered by one frame), then 58 ms with no tick
/// on the relay's clock: the clock's fallback, no request open.
fn live_then_silent(name: &'static str) -> Bench {
    let mut x = Bench::new(name);
    x.tick(2);
    x.b.flush();
    x.relay(2, Entry::Socket);
    x.frame();
    x.relay(3, Entry::Frame);
    x.broker(3);
    assert_eq!(
        x.count(&x.sh.answered),
        1,
        "{name}: one frame answered the request"
    );
    x.relay(60, Entry::Timer);
    assert!(
        !x.relay.tick_live(),
        "{name}: 58 ms without a tick is the fallback"
    );
    x
}

/// How the resumed tick and a fallback frame cross.
#[derive(Clone, Copy, Debug)]
enum Crossing {
    /// The tick is on the socket, unread, when the worker's frame is announced (`relay_thread`
    /// takes the frame before the socket in a turn; QEMU's main loop may do either).
    FrameBeforeRead,
    /// A frame waits for the legacy credit; the credit (`EV_FRAME`) and the tick arrive in ONE
    /// read: the credit sends the frame, then the tick is handled.
    CreditInTheSameRead,
}

fn crossing(name: &'static str, how: Crossing) {
    let mut x = live_then_silent(name);
    match how {
        Crossing::FrameBeforeRead => {
            x.event(EV_FRAME);
            x.b.flush();
            x.relay(60, Entry::Socket);
            x.broker(60);
            x.tick(70);
            x.b.flush();
            x.frame();
            x.relay(70, Entry::Frame);
        }
        Crossing::CreditInTheSameRead => {
            x.frame();
            x.relay(60, Entry::Frame);
            x.broker(60);
            x.event(EV_FRAME);
            x.tick(70);
            x.b.flush();
            x.relay(70, Entry::Socket);
        }
    }
    x.broker(70);
    x.relay(70, Entry::Socket);
    assert!(
        x.relay.tick_live(),
        "{name} ({how:?}): the relay read the resumed tick, after it sent the frame"
    );
    assert_eq!(
        x.count(&x.sh.unsolicited),
        1,
        "{name} ({how:?}): the fallback frame reached the broker as COMMIT.seq 0 after its tick"
    );
    let v = x.sh.violations.lock().unwrap();
    assert!(
        v.is_empty(),
        "{name} ({how:?}): {} violation(s): {:#?}",
        v.len(),
        *v
    );
}

/// ★ EXPECTED RED until the oracle judges by what the relay had READ (M3, 2026-10-23): the resumed
/// tick is unread when the relay sends a frame on the legacy credit, so the broker receives
/// `COMMIT.seq 0` "0 ms after the last tick". The relay could not have known: no violation.
#[test]
fn lockstep_crossing_frame_before_the_tick_is_read() {
    crossing(
        "lockstep_crossing_frame_before_the_tick_is_read",
        Crossing::FrameBeforeRead,
    );
}

/// ★ EXPECTED RED until the oracle judges by what the relay had READ (M3, 2026-10-23): the credit
/// and the resumed tick in one read; the credit sends the waiting frame before the tick is handled.
#[test]
fn lockstep_crossing_credit_and_tick_in_one_read() {
    crossing(
        "lockstep_crossing_credit_and_tick_in_one_read",
        Crossing::CreditInTheSameRead,
    );
}
