// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ The relay's connection machine against a scripted link and a deterministic clock — every
//! invariant of `docs/design/V3_DISPLAY.md` §8.1 and each fix over nvkvm-pv, one test each.

use kf_broker::wire::{
    CAP_DMABUF, CAP_FOCUS_EVENTS, CAP_MODIFIERS, CAP_RELEASE, CMD_ATTACH, CMD_CAPS, CMD_COMMIT,
    CMD_F_SHM, CMD_QUERY_FORMAT, CMD_SIZE, CMD_WINDOW, Cmd, EV_ABS, EV_BTN, EV_CLOSE, EV_FORMAT,
    EV_FRAME, EV_GRAB, EV_HELLO, EV_KEY, EV_REL, EV_RELEASE, EV_SURFACE, EV_WHEEL, FOURCC_XR24,
    MOD_INVALID, MOD_LINEAR, PKT_SIZE, Pkt,
};
use kf_broker::{FrameGeom, FrameRing, Host, Input, Link, Recv, Relay, RelayConfig, Sent, SlotFds};
use kf_linux_raw::{SharedRam, fd_inode};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

#[derive(Default)]
struct Wire {
    peer_uid: u32,
    /// What `effective_uid` answers NOW (a VMM that dropped privileges changes it).
    euid: u32,
    euid_reads: u32,
    connect_err: Option<String>,
    connects: u32,
    inbox: VecDeque<u8>,
    eof: bool,
    recv_calls: u32,
    sent: Vec<(Cmd, Option<u64>)>,
    /// Sends allowed before the socket reports Full (`None`: unlimited).
    budget: Option<usize>,
    open: i32,
    /// ★ Runs after a record of this type was sent — what another thread does in the window
    /// between two of the relay's sends (the worker's `withdraw_all`, for one).
    after_send: Option<(u16, Box<dyn FnMut()>)>,
}

#[derive(Clone)]
struct Fake(Rc<RefCell<Wire>>);

struct Sock(Rc<RefCell<Wire>>);

impl Drop for Sock {
    fn drop(&mut self) {
        self.0.borrow_mut().open -= 1;
    }
}

impl Link for Fake {
    type Sock = Sock;
    fn connect(&mut self, _path: &Path) -> Result<Sock, String> {
        let mut w = self.0.borrow_mut();
        w.connects += 1;
        if let Some(e) = &w.connect_err {
            return Err(e.clone());
        }
        w.open += 1;
        Ok(Sock(self.0.clone()))
    }
    fn fd(&self, _s: &Sock) -> i32 {
        77
    }
    fn peer_uid(&mut self, _s: &Sock) -> Result<u32, String> {
        Ok(self.0.borrow().peer_uid)
    }
    fn effective_uid(&mut self) -> u32 {
        let mut w = self.0.borrow_mut();
        w.euid_reads += 1;
        w.euid
    }
    fn send(&mut self, _s: &Sock, rec: &[u8; CMD_SIZE], fd: Option<BorrowedFd<'_>>) -> Sent {
        let mut w = self.0.borrow_mut();
        if let Some(b) = w.budget.as_mut() {
            if *b == 0 {
                return Sent::Full;
            }
            *b -= 1;
        }
        let id = fd.map(|f| fd_inode(f).expect("a live descriptor"));
        let cmd = Cmd::decode(rec).expect("reserved1 zero");
        w.sent.push((cmd, id));
        let hook = match w.after_send.take() {
            Some((ty, f)) if ty == cmd.ty => Some(f),
            other => {
                w.after_send = other;
                None
            }
        };
        drop(w);
        if let Some(mut f) = hook {
            f();
        }
        Sent::Done
    }
    fn recv(&mut self, _s: &Sock, buf: &mut [u8]) -> Recv {
        let mut w = self.0.borrow_mut();
        w.recv_calls += 1;
        if w.inbox.is_empty() {
            return if w.eof { Recv::Closed } else { Recv::Empty };
        }
        let n = buf.len().min(w.inbox.len());
        for b in buf.iter_mut().take(n) {
            *b = w.inbox.pop_front().unwrap();
        }
        Recv::Bytes(n)
    }
}

#[derive(Default)]
struct H {
    watches: Vec<(i32, bool, bool)>,
    timer: Option<u64>,
}

impl Host for H {
    fn watch(&mut self, fd: i32, read: bool, write: bool) {
        self.watches.push((fd, read, write));
    }
    fn timer(&mut self, d: Option<u64>) {
        self.timer = d;
    }
}

const ME: u32 = 1000;

struct T {
    wire: Rc<RefCell<Wire>>,
    relay: Relay<Fake>,
    ring: Arc<FrameRing>,
    host: H,
    now: u64,
    serial: u64,
}

impl T {
    /// A relay over a five-slot ring whose slots carry a memfd and (`dmabuf`) a second
    /// descriptor standing in for the udmabuf (a distinct inode, as the real one has).
    fn new(dmabuf: bool) -> T {
        let ring = Arc::new(FrameRing::new(kf_broker::slots::BROKER_SLOTS, true));
        for j in 0..ring.slots() {
            let mem = SharedRam::create_named(c"kfb-test-frame", 64 * 1024).unwrap();
            let dma = dmabuf.then(|| {
                SharedRam::create_named(c"kfb-test-dmabuf", 64 * 1024)
                    .unwrap()
                    .dup_for_export()
                    .unwrap()
            });
            ring.install(j, SlotFds::new(mem, dma).unwrap()).unwrap();
        }
        let wire = Rc::new(RefCell::new(Wire {
            peer_uid: ME,
            euid: ME,
            ..Wire::default()
        }));
        let relay = Relay::new(
            RelayConfig {
                path: PathBuf::from("/run/test/display.sock"),
                extra_uid: None,
            },
            ring.clone(),
            Fake(wire.clone()),
        );
        T {
            wire,
            relay,
            ring,
            host: H::default(),
            now: 1_000,
            serial: 0,
        }
    }

    fn publish(&mut self, w: u32, h: u32) -> usize {
        self.publish_fourcc(w, h, FOURCC_XR24)
    }

    fn publish_fourcc(&mut self, w: u32, h: u32, fourcc: u32) -> usize {
        let t = self.ring.fill_target(None).expect("a fill target");
        self.serial += 1;
        self.ring.describe(
            t,
            FrameGeom {
                width: w,
                height: h,
                stride: w * 4,
                fourcc,
                serial: self.serial,
            },
        );
        self.ring.publish(t);
        t
    }

    fn frame(&mut self) {
        let now = self.now;
        self.relay.on_frame(now, &mut self.host);
    }

    fn pkt(&self, ty: u16, x: i32, y: i32, w0: u32, w1: u32) {
        let p = Pkt {
            ty,
            x,
            y,
            w0,
            w1,
            ..Pkt::default()
        };
        self.wire.borrow_mut().inbox.extend(p.encode());
    }

    fn read(&mut self) -> Vec<Input> {
        let mut out = Vec::new();
        let now = self.now;
        self.relay
            .on_socket(now, true, false, &mut self.host, &mut out, 64);
        out
    }

    fn writable(&mut self) {
        let now = self.now;
        let mut out = Vec::new();
        self.relay
            .on_socket(now, false, true, &mut self.host, &mut out, 64);
    }

    fn tick(&mut self, ms: u64) {
        self.now += ms;
        let now = self.now;
        self.relay.on_timer(now, &mut self.host);
    }

    /// `start` arms the first attempt for now; the VMM's loop then fires the timer.
    fn start(&mut self) {
        let now = self.now;
        self.relay.start(now, &mut self.host);
        assert_eq!(
            self.host.timer,
            Some(now),
            "the first attempt is armed for now"
        );
        self.relay.on_timer(now, &mut self.host);
    }

    /// Connect and complete the HELLO with `caps` (always including `CAP_DMABUF`), keeping what
    /// the replay sent.
    fn up_keep(&mut self, caps: u32) {
        self.start();
        self.pkt(EV_HELLO, 0, 0, 2, caps | CAP_DMABUF);
        self.read();
        assert!(self.relay.active(), "HELLO with CAPS must reach ACTIVE");
    }

    /// As [`T::up_keep`], then forget the replay's records.
    fn up(&mut self, caps: u32) {
        self.up_keep(caps);
        self.clear();
    }

    /// The broker is done with the frame in `slot` (its memfd was sent: the shm rung).
    fn release_shm(&self, slot: usize) {
        let id = self.memfd_id(slot);
        self.pkt(EV_RELEASE, 0, 0, id as u32, (id >> 32) as u32);
    }

    fn sent(&self) -> Vec<(Cmd, Option<u64>)> {
        self.wire.borrow().sent.clone()
    }

    fn types(&self) -> Vec<u16> {
        self.sent().iter().map(|(c, _)| c.ty).collect()
    }

    fn clear(&self) {
        self.wire.borrow_mut().sent.clear();
    }

    fn memfd_id(&self, j: usize) -> u64 {
        self.ring.fds(j).unwrap().memfd_id()
    }

    fn dmabuf_id(&self, j: usize) -> u64 {
        self.ring.fds(j).unwrap().dmabuf_id().unwrap()
    }
}

// ── handshake ──────────────────────────────────────────────────────────────────────────────

/// ★ Invariant 2: HELLO first, version 2, `CAP_DMABUF` required — each refusal is a failed
/// attempt (closed, retried), never a half-open connection.
#[test]
fn hello_must_come_first_with_version_2_and_dmabuf() {
    for (ty, ver, caps, why) in [
        (EV_FRAME, 2, CAP_DMABUF, "not HELLO first"),
        (EV_HELLO, 1, CAP_DMABUF, "version 1"),
        (EV_HELLO, 2, 0, "no CAP_DMABUF"),
    ] {
        let mut t = T::new(false);
        t.start();
        t.pkt(ty, 0, 0, ver, caps);
        t.read();
        assert!(!t.relay.connected(), "{why}: the attempt must fail");
        assert_eq!(t.relay.counters().attempts_failed, 1, "{why}");
        assert_eq!(t.wire.borrow().open, 0, "{why}: the socket is closed");
        assert_eq!(
            t.host.watches.last(),
            Some(&(77, false, false)),
            "{why}: unwatched before close"
        );
        assert!(t.host.timer.is_some(), "{why}: a retry is scheduled");
    }
    let mut t = T::new(false);
    t.up_keep(0);
    assert_eq!(
        t.types(),
        vec![CMD_CAPS],
        "no frame yet: the replay is CAPS alone"
    );
    assert_eq!(t.sent()[0].0.width, 0, "CAPS without the clipboard bit");
}

/// ★ Fix (c): the peer check runs right after connect, BEFORE a byte is read.
#[test]
fn the_peer_check_runs_before_any_read() {
    let mut t = T::new(false);
    t.wire.borrow_mut().peer_uid = 65534;
    t.pkt(EV_HELLO, 0, 0, 2, CAP_DMABUF); // a squatter that would even speak the protocol
    t.start();
    t.read();
    assert!(!t.relay.connected());
    assert_eq!(
        t.wire.borrow().recv_calls,
        0,
        "not one byte read from a refused peer"
    );
    let c = t.relay.counters();
    assert_eq!((c.peer_refused, c.attempts_failed), (1, 1));
}

/// A relay whose broker runs as `peer`, in a VMM whose euid is `euid`, with
/// `display-broker-uid` = `extra`: started and given a HELLO. Returns whether it went ACTIVE.
fn admitted(peer: u32, euid: u32, extra: Option<u32>) -> bool {
    let ring = Arc::new(FrameRing::new(kf_broker::slots::BROKER_SLOTS, true));
    let wire = Rc::new(RefCell::new(Wire {
        peer_uid: peer,
        euid,
        ..Wire::default()
    }));
    let mut relay = Relay::new(
        RelayConfig {
            path: PathBuf::from("/tmp/kf3/display.sock"),
            extra_uid: extra,
        },
        ring,
        Fake(wire.clone()),
    );
    let mut host = H::default();
    relay.start(0, &mut host);
    relay.on_timer(0, &mut host);
    let hello = Pkt {
        ty: EV_HELLO,
        w0: 2,
        w1: CAP_DMABUF,
        ..Pkt::default()
    };
    wire.borrow_mut().inbox.extend(hello.encode());
    relay.on_socket(0, true, false, &mut host, &mut Vec::new(), 64);
    relay.active()
}

/// ★ The peer policy (corrected 2026-10-03, the review of `v3-broker`): exactly uid 0, the
/// VMM's effective uid and `display-broker-uid`. ⊘ Nothing else — in particular not the owner of
/// the socket's directory, which anyone can be when the directory is missing (`mkdir /tmp/kf3`
/// after a reboot); the loopback's `a_squatter_who_owns_the_sockets_directory_is_refused` runs
/// that case on a real socket.
#[test]
fn only_root_the_vmms_euid_and_display_broker_uid_are_admitted() {
    assert!(admitted(0, ME, None), "root");
    assert!(admitted(ME, ME, None), "the VMM's own uid");
    assert!(admitted(4242, ME, Some(4242)), "display-broker-uid");
    assert!(!admitted(4242, ME, None), "any other uid");
    assert!(
        !admitted(4242, ME, Some(4243)),
        "a different display-broker-uid"
    );
    assert!(
        !admitted(ME, 0, None),
        "a root VMM does not admit a user's broker by itself"
    );
    assert!(
        admitted(ME, 0, Some(ME)),
        "...unless display-broker-uid names it"
    );
}

/// ★ The euid is read at EACH attempt — QEMU realizes devices as root, then `-run-with user=`
/// (or `-runas`) drops to the user before its main loop runs. `start` makes no attempt of its
/// own: the first one runs from the timer, after the drop, and a broker running as that user is
/// admitted. (Read once at realize — the review's finding — it named root and refused it.)
#[test]
fn the_euid_is_read_at_each_connect_after_privileges_are_dropped() {
    let mut t = T::new(false);
    t.wire.borrow_mut().euid = 0; // realize: QEMU still runs as root
    let now = t.now;
    t.relay.start(now, &mut t.host);
    assert_eq!(t.wire.borrow().connects, 0, "start never connects");
    assert_eq!(t.wire.borrow().euid_reads, 0, "nor reads the euid");
    assert_eq!(
        t.host.timer,
        Some(now),
        "the first attempt is the timer's, at once"
    );
    t.wire.borrow_mut().euid = ME; // os_setup_post: setuid(ME), then the main loop
    t.relay.on_timer(now, &mut t.host);
    t.pkt(EV_HELLO, 0, 0, 2, CAP_DMABUF);
    t.read();
    assert!(t.relay.active(), "the broker running as ME is admitted");
    assert_eq!(t.relay.counters().peer_refused, 0);
    // and again at a reconnect: the euid of THAT attempt
    t.wire.borrow_mut().eof = true;
    t.read();
    assert!(!t.relay.connected());
    t.wire.borrow_mut().eof = false;
    t.wire.borrow_mut().euid = 4242;
    t.tick(200);
    assert_eq!(
        t.relay.counters().peer_refused,
        1,
        "judged against the euid of now"
    );
    assert_eq!(t.wire.borrow().euid_reads, 2);
}

/// ★ No CONNECTING state: a connect that fails (EAGAIN included) is a failed attempt at once.
/// ★ Invariant 4: backoff doubles from 200 ms to 5 s, loud once.
#[test]
fn a_failed_connect_backs_off_200_400_to_5000() {
    let mut t = T::new(false);
    t.wire.borrow_mut().connect_err = Some("errno 11 (EAGAIN: backlog full)".into());
    t.start();
    assert!(!t.relay.connected());
    let mut waits = Vec::new();
    for _ in 0..8 {
        let at = t.host.timer.expect("a retry is always scheduled");
        waits.push(at - t.now);
        t.now = at;
        let now = t.now;
        t.relay.on_timer(now, &mut t.host);
    }
    assert_eq!(waits, vec![200, 400, 800, 1600, 3200, 5000, 5000, 5000]);
    assert_eq!(t.wire.borrow().connects, 9);
    // the broker appears: the next retry connects, and the backoff resets on ACTIVE
    t.wire.borrow_mut().connect_err = None;
    let at = t.host.timer.unwrap();
    t.now = at;
    t.pkt(EV_HELLO, 0, 0, 2, CAP_DMABUF);
    let now = t.now;
    t.relay.on_timer(now, &mut t.host);
    t.read();
    assert!(t.relay.active());
}

/// ★ Invariant 3: connect + HELLO + replay must finish within 2 s.
#[test]
fn the_handshake_has_a_two_second_limit() {
    let mut t = T::new(false);
    t.start();
    assert!(t.relay.connected());
    assert_eq!(t.host.timer, Some(t.now + 2000));
    t.tick(1999);
    assert!(t.relay.connected());
    t.tick(1);
    assert!(
        !t.relay.connected(),
        "a silent broker is dropped at the deadline"
    );
    assert_eq!(t.relay.counters().attempts_failed, 1);
}

/// ★ Invariant 1: a short read builds up to exactly one packet; nothing resyncs.
#[test]
fn a_short_read_builds_up_to_24_bytes() {
    let mut t = T::new(false);
    t.start();
    let hello = Pkt {
        ty: EV_HELLO,
        w0: 2,
        w1: CAP_DMABUF,
        ..Pkt::default()
    }
    .encode();
    t.wire.borrow_mut().inbox.extend(&hello[..10]);
    t.read();
    assert!(!t.relay.active(), "10 bytes are not a packet");
    t.wire.borrow_mut().inbox.extend(&hello[10..]);
    t.read();
    assert!(t.relay.active(), "the next 14 complete it");
}

/// ★ At most 64 packets per call; the tail stays in the socket for the next level-triggered
/// wakeup (no decoded packet is stranded in a buffer nobody is woken for).
#[test]
fn at_most_64_packets_per_call_and_the_tail_stays_in_the_socket() {
    let mut t = T::new(false);
    t.up(0);
    for i in 0..100 {
        t.pkt(EV_KEY, 30, i % 2, 0, 0);
    }
    let first = t.read();
    assert_eq!(first.len(), 64);
    assert_eq!(
        t.wire.borrow().inbox.len(),
        36 * PKT_SIZE,
        "the tail was not read"
    );
    let second = t.read();
    assert_eq!(second.len(), 36);
    // a partial packet is carried, never more than one
    t.pkt(EV_KEY, 31, 1, 0, 0);
    t.wire.borrow_mut().inbox.truncate(PKT_SIZE - 3);
    assert!(t.read().is_empty());
    t.wire.borrow_mut().inbox.extend([0u8; 3]);
    assert_eq!(
        t.read(),
        vec![Input::Key {
            code: 31,
            down: true
        }]
    );
}

// ── frames ────────────────────────────────────────────────────────────────────────────────

/// ★ Invariants 5 and 13: replay order WINDOW → ATTACH → COMMIT → CAPS, and the ATTACH flags
/// (F_SHM) travel with the replayed frame.
#[test]
fn the_replay_is_window_attach_commit_caps_with_the_frames_flags() {
    let mut t = T::new(false); // no udmabuf: the shared-memory rung
    let j = t.publish(64, 32);
    t.up_keep(0);
    let s = t.sent();
    assert_eq!(
        t.types(),
        vec![CMD_WINDOW, CMD_ATTACH, CMD_COMMIT, CMD_CAPS],
        "{s:?}"
    );
    assert_eq!((s[0].0.width, s[0].0.height), (64, 32));
    let a = s[1].0;
    assert_eq!(
        a.flags, CMD_F_SHM,
        "the replay carries F_SHM (nvkvm-pv's did not)"
    );
    assert_eq!((a.width, a.height, a.stride, a.offset), (64, 32, 256, 0));
    assert_eq!(s[1].1, Some(t.memfd_id(j)), "the memfd went");
    assert_eq!(t.relay.counters().sent, 1);
}

/// ★ Invariant 6: a newer frame while the replay's COMMIT is owed restarts the replay.
#[test]
fn a_newer_frame_during_the_replay_commit_restarts_the_replay() {
    let mut t = T::new(false);
    t.publish(64, 32);
    t.start();
    t.wire.borrow_mut().budget = Some(2); // WINDOW and ATTACH go, COMMIT is Full
    t.pkt(EV_HELLO, 0, 0, 2, CAP_DMABUF);
    t.read();
    assert!(t.relay.connected() && !t.relay.active());
    assert_eq!(
        t.host.watches.last(),
        Some(&(77, true, true)),
        "waits for POLLOUT"
    );
    t.publish(64, 32);
    t.frame();
    assert!(!t.relay.connected(), "closed, to replay from the start");
    assert_eq!(t.relay.counters().attempts_failed, 1);
}

/// ★ Invariant 7: EAGAIN on ATTACH owes the ATTACH; EAGAIN on COMMIT owes ONLY the COMMIT
/// (never a second import); retried on writability, with a 50 ms backstop.
#[test]
fn an_owed_attach_and_an_owed_commit_are_different_debts() {
    let mut t = T::new(false);
    t.up(0);
    t.publish(64, 32);
    t.clear();
    t.wire.borrow_mut().budget = Some(1); // WINDOW goes, ATTACH is Full
    t.frame();
    assert_eq!(t.types(), vec![CMD_WINDOW]);
    assert_eq!(t.relay.counters().dropped, 1);
    assert_eq!(t.host.watches.last(), Some(&(77, true, true)));
    t.wire.borrow_mut().budget = None;
    t.writable();
    assert_eq!(t.types(), vec![CMD_WINDOW, CMD_ATTACH, CMD_COMMIT]);
    assert_eq!(t.relay.counters().recovered, 1);
    assert_eq!(
        t.host.watches.last(),
        Some(&(77, true, false)),
        "POLLOUT disarmed"
    );
    // EAGAIN on COMMIT
    t.pkt(EV_FRAME, 0, 0, 0, 0);
    t.read();
    t.publish(64, 32);
    t.clear();
    t.wire.borrow_mut().budget = Some(1); // ATTACH goes (no WINDOW: same size), COMMIT is Full
    t.frame();
    assert_eq!(t.types(), vec![CMD_ATTACH]);
    assert_eq!(t.relay.counters().uncommitted, 1);
    t.wire.borrow_mut().budget = None;
    t.tick(49);
    assert_eq!(t.types(), vec![CMD_ATTACH], "not before the 50 ms backstop");
    t.tick(1);
    assert_eq!(
        t.types(),
        vec![CMD_ATTACH, CMD_COMMIT],
        "the backstop re-sends ONLY the commit"
    );
}

/// ★ Invariant 7: a stale WINDOW is re-sent before an owed ATTACH; a newer frame replaces what
/// is owed (an owed ATTACH's slot goes back: the broker never heard of it).
#[test]
fn a_newer_frame_replaces_an_owed_one() {
    let mut t = T::new(false);
    t.up(0);
    let a = t.publish(64, 32);
    t.wire.borrow_mut().budget = Some(0); // even the WINDOW is Full
    t.frame();
    assert_eq!(t.ring.held_mask(), 1 << a);
    let b = t.publish(128, 64);
    t.wire.borrow_mut().budget = None;
    t.clear();
    t.frame();
    assert_eq!(
        t.ring.held_mask(),
        1 << b,
        "the owed frame's slot came back"
    );
    let s = t.sent();
    assert_eq!(t.types(), vec![CMD_WINDOW, CMD_ATTACH, CMD_COMMIT]);
    assert_eq!(
        (s[0].0.width, s[1].0.width),
        (128, 128),
        "the NEWER frame went"
    );
}

/// ★ Invariant 9: WINDOW only when the size changes.
#[test]
fn window_is_sent_only_when_the_size_changes() {
    let mut t = T::new(false);
    t.up(CAP_RELEASE);
    for (w, h) in [(64, 32), (64, 32), (128, 32), (128, 32)] {
        let j = t.publish(w, h);
        t.frame();
        t.release_shm(j);
        t.pkt(EV_FRAME, 0, 0, 0, 0);
        t.read();
    }
    assert_eq!(t.relay.counters().sent, 4);
    let windows = t.types().iter().filter(|&&ty| ty == CMD_WINDOW).count();
    assert_eq!(windows, 2);
}

/// ★ Fix (d): a COMMIT spends the credit; FRAME returns it, so does the RELEASE of the latest
/// commit (X11 XRender sends no FRAME), and the 100 ms backstop is the last resort.
#[test]
fn pacing_credit_returns_on_frame_release_or_the_backstop() {
    let mut t = T::new(false);
    t.up(CAP_RELEASE);
    let a = t.publish(64, 32);
    t.frame();
    t.clear();
    let b = t.publish(64, 32);
    t.frame();
    assert!(
        t.sent().is_empty(),
        "no credit: the frame waits (latest wins)"
    );
    t.pkt(EV_FRAME, 0, 0, 0, 0);
    t.read();
    assert_eq!(
        t.types(),
        vec![CMD_ATTACH, CMD_COMMIT],
        "FRAME returned the credit"
    );
    assert_eq!(t.ring.held_mask(), (1 << a) | (1 << b));
    // the RELEASE of an OLDER frame frees its slot but returns no credit
    t.release_shm(a);
    t.read();
    t.clear();
    let c = t.publish(64, 32);
    t.frame();
    assert!(
        t.sent().is_empty(),
        "a's release is not the latest commit's"
    );
    // the RELEASE of the LATEST commit returns it (X11 XRender sends no FRAME)
    t.release_shm(b);
    t.read();
    assert_eq!(
        t.types(),
        vec![CMD_ATTACH, CMD_COMMIT],
        "RELEASE of the latest commit"
    );
    assert_eq!(t.ring.held_mask(), 1 << c);
    // nothing from the broker at all: the backstop
    t.clear();
    t.publish(64, 32);
    t.frame();
    assert!(t.sent().is_empty());
    t.tick(99);
    assert!(t.sent().is_empty());
    t.tick(1);
    assert_eq!(
        t.types(),
        vec![CMD_ATTACH, CMD_COMMIT],
        "the 100 ms backstop"
    );
    assert_eq!(t.relay.counters().backstops, 1);
}

/// ★ RELEASE is matched by the id of the descriptor actually SENT (the dma-buf's on the dma-buf
/// rung); an unknown id is counted and ignored.
#[test]
fn release_matches_the_sent_id_and_ignores_unknown_ones() {
    let mut t = T::new(true);
    t.up(CAP_MODIFIERS);
    let j = t.publish(64, 32);
    t.frame();
    let s = t.sent();
    let attach = s.iter().find(|(c, _)| c.ty == CMD_ATTACH).unwrap();
    assert_eq!(attach.0.modifier, MOD_LINEAR);
    assert_eq!(
        attach.1,
        Some(t.dmabuf_id(j)),
        "the dma-buf went, not the memfd"
    );
    // the memfd's id names nothing that was sent
    let m = t.memfd_id(j);
    t.pkt(EV_RELEASE, 0, 0, m as u32, (m >> 32) as u32);
    t.read();
    assert_eq!(t.relay.counters().unknown_releases, 1);
    assert_eq!(t.ring.held_mask(), 1 << j);
    let d = t.dmabuf_id(j);
    t.pkt(EV_RELEASE, 0, 0, d as u32, (d >> 32) as u32);
    t.read();
    assert_eq!(t.relay.counters().releases, 1);
    assert_eq!(
        t.ring.held_mask(),
        1 << j,
        "the latest commit stays RETAINED (the replay frame) after its RELEASE"
    );
    let k = t.publish(64, 32);
    t.frame();
    assert_eq!(
        t.ring.held_mask(),
        1 << k,
        "a newer commit lets the released one go"
    );
}

/// ★ Invariant 11 + fix (a): EV_FORMAT is matched by the echoed pair (an untracked pair is
/// ignored); a LATER "no" downgrades a "yes" and reclaims the frames attached under that pair.
#[test]
fn a_later_no_downgrades_the_rung_and_reclaims_its_frames() {
    let mut t = T::new(true);
    t.up(CAP_MODIFIERS | CAP_RELEASE);
    let j = t.publish(64, 32);
    t.frame();
    assert_eq!(
        t.types(),
        vec![CMD_QUERY_FORMAT, CMD_WINDOW, CMD_ATTACH, CMD_COMMIT],
        "asked once, and optimistic while unanswered"
    );
    let q = t.sent()[0].0;
    assert_eq!((q.fourcc, q.modifier), (FOURCC_XR24, MOD_LINEAR));
    t.pkt(EV_FORMAT, 1, FOURCC_XR24 as i32, 0x1234, 0); // a pair we never asked about
    t.read();
    t.pkt(EV_FORMAT, 1, FOURCC_XR24 as i32, 0, 0);
    t.read();
    assert_eq!(t.ring.held_mask(), 1 << j, "a yes changes nothing held");
    // the unsolicited probe failure
    t.pkt(EV_FORMAT, 0, FOURCC_XR24 as i32, 0, 0);
    t.read();
    assert_eq!(
        t.ring.held_mask(),
        0,
        "the frame attached under the refused pair came back"
    );
    assert_eq!(t.relay.counters().reclaims, 1);
    t.clear();
    let k = t.publish(64, 32);
    t.frame();
    let s = t.sent();
    let a = s.iter().find(|(c, _)| c.ty == CMD_ATTACH).expect("sent");
    assert_eq!(a.0.flags, CMD_F_SHM, "downgraded to shared memory");
    assert_eq!(a.1, Some(t.memfd_id(k)));
    assert!(
        !t.types().contains(&CMD_QUERY_FORMAT),
        "asked once per connection"
    );
}

/// ★ Rung 1b: a broker without CAP_MODIFIERS is asked about (XR24, MOD_INVALID); until it says
/// yes frames go as shared memory, then as the dma-buf with the implicit modifier.
#[test]
fn rung_1b_is_the_implicit_modifier_after_an_explicit_yes() {
    let mut t = T::new(true);
    t.up(CAP_RELEASE);
    t.publish(64, 32);
    t.frame();
    let s = t.sent();
    assert_eq!(s[0].0.ty, CMD_QUERY_FORMAT);
    assert_eq!(s[0].0.modifier, MOD_INVALID);
    assert_eq!(s[2].0.flags, CMD_F_SHM, "no answer yet: shared memory");
    t.pkt(
        EV_FORMAT,
        1,
        FOURCC_XR24 as i32,
        MOD_INVALID as u32,
        (MOD_INVALID >> 32) as u32,
    );
    t.pkt(EV_FRAME, 0, 0, 0, 0);
    t.read();
    t.clear();
    let j = t.publish(64, 32);
    t.frame();
    let s = t.sent();
    let a = s.iter().find(|(c, _)| c.ty == CMD_ATTACH).unwrap();
    assert_eq!((a.0.flags, a.0.modifier), (0, MOD_INVALID));
    assert_eq!(a.1, Some(t.dmabuf_id(j)));
}

/// ★ The narrowed reclaim rule: a held frame with no RELEASE comes back 1 s after its commit
/// once a newer frame was committed — the rejected-ATTACH case, which never gets a RELEASE.
#[test]
fn a_frame_without_a_release_is_reclaimed_after_a_second_and_a_newer_commit() {
    let mut t = T::new(false);
    t.up(0);
    let a = t.publish(64, 32);
    t.frame();
    t.pkt(EV_FRAME, 0, 0, 0, 0);
    t.read();
    let b = t.publish(64, 32);
    t.frame();
    assert_eq!(t.ring.held_mask(), (1 << a) | (1 << b), "two held: the cap");
    t.pkt(EV_FRAME, 0, 0, 0, 0);
    t.read();
    t.publish(64, 32);
    t.frame();
    assert_eq!(t.relay.counters().blocked, 1, "the third waits on the cap");
    assert_eq!(t.relay.counters().sent, 2);
    let at = t.host.timer.expect("a reclaim deadline");
    t.now = at;
    let now = t.now;
    t.relay.on_timer(now, &mut t.host);
    assert_eq!(t.relay.counters().reclaims, 1);
    assert_eq!(t.relay.counters().sent, 3, "the waiting frame went");
    assert_eq!(
        t.ring.held_mask() & (1 << a),
        0,
        "the OLDEST came back, not the latest"
    );
    assert_ne!(t.ring.held_mask() & (1 << b), 0);
}

/// ★ Invariant 8: per-connection state is dropped on disconnect — WINDOW and the verdicts are
/// re-learned — while the latest frame is kept and replayed.
#[test]
fn a_reconnect_forgets_the_connection_and_replays_the_latest_frame() {
    let mut t = T::new(true);
    t.up(CAP_MODIFIERS);
    t.publish(64, 32);
    t.frame();
    // LINEAR is refused on THIS connection: the frame comes back, the next goes as shm
    t.pkt(EV_FORMAT, 0, FOURCC_XR24 as i32, 0, 0);
    t.read();
    let k = t.publish(64, 32);
    t.frame();
    let s = t.sent();
    assert_eq!(s.last().unwrap().0.ty, CMD_COMMIT);
    assert_eq!(s[s.len() - 2].0.flags, CMD_F_SHM);
    t.wire.borrow_mut().eof = true;
    t.read();
    assert!(!t.relay.connected());
    assert_eq!(t.ring.held_mask(), 0, "disconnect clears the held set");
    assert_eq!(
        t.ring.broker_ready(),
        Some(k),
        "the latest commit waits for the replay"
    );
    t.wire.borrow_mut().eof = false;
    t.clear();
    let at = t.host.timer.unwrap();
    assert_eq!(
        at - t.now,
        200,
        "an ACTIVE connection's loss retries at the shortest delay"
    );
    t.now = at;
    t.pkt(EV_HELLO, 0, 0, 2, CAP_DMABUF | CAP_MODIFIERS);
    let now = t.now;
    t.relay.on_timer(now, &mut t.host);
    t.read();
    assert!(t.relay.active());
    assert_eq!(
        t.types(),
        vec![
            CMD_QUERY_FORMAT,
            CMD_WINDOW,
            CMD_ATTACH,
            CMD_COMMIT,
            CMD_CAPS
        ],
        "asked again, WINDOW again, and the last frame replayed"
    );
    let s = t.sent();
    assert_eq!(
        (s[2].0.flags, s[2].0.modifier),
        (0, MOD_LINEAR),
        "the old 'no' was forgotten"
    );
    assert_eq!(s[2].1, Some(t.dmabuf_id(k)), "slot {k} replayed");
    assert_eq!(t.relay.counters().reconnects, 1);
}

/// ★ Invariants 10, 12 and the input bounds (§8.4): unknown types skipped exactly; keys and
/// buttons inside evdev's range; ABS clamped and dropped for a zero range; REL summed; the
/// horizontal wheel dropped; GRAB and CLOSE handed over.
#[test]
fn input_is_bounded_before_the_vmm_sees_it() {
    let mut t = T::new(false);
    t.up(CAP_FOCUS_EVENTS);
    t.pkt(999, 1, 2, 3, 4); // a type from a newer broker
    t.pkt(EV_KEY, 30, 1, 0, 0);
    t.pkt(EV_KEY, -1, 1, 0, 0);
    t.pkt(EV_KEY, 0x300, 1, 0, 0);
    t.pkt(EV_BTN, 0x110, 0, 0, 0);
    t.pkt(EV_ABS, 5000, -3, 1920, 1080);
    t.pkt(EV_ABS, 5, 5, 0, 1080);
    t.pkt(EV_REL, i32::MAX, 1, 0, 0);
    t.pkt(EV_REL, 5, 2, 0, 0);
    t.pkt(EV_WHEEL, -1, 0, 0, 0);
    t.pkt(EV_WHEEL, 0, 3, 0, 0);
    t.pkt(EV_GRAB, 1, 0, 0, 0);
    t.pkt(EV_CLOSE, 0, 0, 0, 0);
    t.pkt(EV_CLOSE, 0, 0, 0, 0);
    t.pkt(EV_CLOSE, 1, 0, 0, 0);
    t.pkt(EV_SURFACE, 1600, 900, 59_940, 0);
    t.pkt(EV_SURFACE, 1600, 900, 59_940, 0); // the same size again: no second hint
    t.pkt(EV_SURFACE, 20_000, 10, 0, 0); // clamped
    assert_eq!(
        t.read(),
        vec![
            Input::Key {
                code: 30,
                down: true
            },
            Input::Btn {
                code: 0x110,
                down: false
            },
            Input::Abs {
                x: 1919,
                y: 0,
                w: 1920,
                h: 1080
            },
            Input::Rel {
                dx: i32::MAX,
                dy: 3
            },
            Input::Wheel { up: false },
            Input::Grab(true),
            Input::Close { force: false },
            Input::Close { force: false },
            Input::Close { force: true },
            Input::Surface {
                w: 1600,
                h: 900,
                mhz: 59_940
            },
            Input::Surface {
                w: 8192,
                h: 64,
                mhz: 0
            },
        ]
    );
    assert!(t.relay.active(), "nothing above is a protocol violation");
}

/// ★ Fds stay valid: a frame on the dma-buf rung uses the slot's CURRENT generation, and the
/// relay never sends a frame whose geometry its backing cannot hold.
#[test]
fn a_frame_larger_than_its_backing_is_refused_before_the_wire() {
    let mut t = T::new(false);
    t.up(0);
    t.publish(4096, 4096); // 64 MiB of pixels in a 64 KiB backing
    t.frame();
    assert!(t.sent().is_empty());
    assert_eq!(t.relay.counters().refused, 1);
    assert_eq!(t.ring.held_mask(), 0);
}

/// ★ The review's freeze (2026-10-03): the broker stalls, the COMMIT after an ATTACH gets
/// EAGAIN (owed), and the next frame supersedes it while the latest commit and the superseded
/// frame fill the cap of 2. The superseded frame is never shown (no COMMIT follows its ATTACH)
/// and never released, and it was neither the latest commit nor reclaimable — so nothing was
/// ever committed again (`sent=1 blocked=61 reclaims=0 held=0b00011 timer=None`). It now yields
/// its slot to the live frame.
#[test]
fn a_superseded_owed_commit_never_freezes_the_display() {
    let mut t = T::new(false);
    t.up(0);
    let a = t.publish(64, 32);
    t.frame(); // a: WINDOW ATTACH COMMIT — the latest commit, never released (on screen)
    t.pkt(EV_FRAME, 0, 0, 0, 0);
    t.read(); // credit back
    let b = t.publish(64, 32);
    t.clear();
    t.wire.borrow_mut().budget = Some(1); // b's ATTACH goes, its COMMIT is Full: owed
    t.frame();
    assert_eq!(t.types(), vec![CMD_ATTACH]);
    assert_eq!(t.relay.counters().uncommitted, 1);
    assert_eq!(
        t.ring.held_mask(),
        (1 << a) | (1 << b),
        "the cap of 2 is full"
    );
    t.wire.borrow_mut().budget = None;
    let c = t.publish(64, 32); // supersedes b's owed COMMIT
    t.frame();
    assert_eq!(
        t.types(),
        vec![CMD_ATTACH, CMD_ATTACH, CMD_COMMIT],
        "c's ATTACH comes before the next COMMIT: b is never shown"
    );
    assert_eq!(t.relay.counters().sent, 2, "c went");
    assert_eq!(
        t.ring.held_mask() & (1 << b),
        0,
        "the superseded frame gave its slot back"
    );
    assert_ne!(t.ring.held_mask() & (1 << c), 0);
    // and frames keep flowing
    for _ in 0..30 {
        t.pkt(EV_FRAME, 0, 0, 0, 0);
        t.read();
        t.publish(64, 32);
        t.frame();
        t.tick(1000);
    }
    let k = t.relay.counters();
    assert_eq!(k.sent, 32, "{k:?}");
    assert!(t.ring.held_mask().count_ones() <= 2);
}

/// ★ "The broker is shown nothing" after a refused backing (the reviews of 2026-10-03): once the
/// worker withdraws the ring (`FrameRing::withdraw_all`), a frame published in a slot that still
/// carries its broker memfd is never sent and makes no line, and the frame the broker held —
/// requeued by its restart — is NOT replayed: `fits()` refuses it, by name. ⊘ This is the one
/// path where the relay's own `broker_backed` check is what stops the frame (the ring offers a
/// requeued slot as broker-ready whatever its backing); remove that check and the replay sends
/// WINDOW → ATTACH → COMMIT of the withdrawn slot.
#[test]
fn a_withdrawn_ring_sends_the_broker_nothing_not_even_a_replay() {
    let mut t = T::new(false);
    t.up(0);
    let k = t.publish(64, 32);
    t.frame();
    assert_eq!(t.types(), vec![CMD_WINDOW, CMD_ATTACH, CMD_COMMIT]);
    t.clear();
    let log = kf_broker::capture_log();
    t.ring.withdraw_all(); // the worker: a broker backing was refused
    let j = t.publish(64, 32);
    assert!(
        t.ring.fds(j).is_some(),
        "slot {j} still carries its broker memfd"
    );
    t.frame();
    assert!(t.sent().is_empty(), "a frame after the refusal was sent");
    assert_eq!(t.relay.counters().refused, 0, "never even broker-ready");
    assert!(log.lines().is_empty(), "{:?}", log.lines());
    // the broker restarts: its held frame is requeued for the replay, as any held frame is ...
    t.wire.borrow_mut().eof = true;
    t.read();
    assert!(!t.relay.connected());
    assert_eq!(t.ring.broker_ready(), Some(k), "requeued for the replay");
    t.wire.borrow_mut().eof = false;
    t.now = t.host.timer.unwrap();
    t.pkt(EV_HELLO, 0, 0, 2, CAP_DMABUF);
    let now = t.now;
    t.relay.on_timer(now, &mut t.host);
    t.read();
    assert!(t.relay.active());
    // ... and refused before the wire: the replay is CAPS alone
    assert_eq!(
        t.types(),
        vec![CMD_CAPS],
        "the withdrawn slot {k} was replayed"
    );
    assert_eq!(t.relay.counters().refused, 1);
    assert_eq!(
        t.ring.held_mask(),
        0,
        "the refused frame gave its slot back"
    );
    let said: Vec<_> = log
        .lines()
        .into_iter()
        .filter(|l| l.contains("REFUSED frame slot"))
        .collect();
    assert_eq!(said.len(), 1, "{:?}", log.lines());
    assert!(said[0].contains("withdrawn"), "{said:?}");
    // and nothing after it either
    for _ in 0..5 {
        t.publish(64, 32);
        t.frame();
        t.tick(200);
    }
    assert_eq!(t.types(), vec![CMD_CAPS]);
}

/// ★ A refused frame is logged for the first four and then every 256th — not once per frame for
/// as long as it lasts (the review's log flood, 2026-10-03).
#[test]
fn a_refused_frame_is_logged_at_a_bounded_rate() {
    let mut t = T::new(false);
    t.up(0);
    let log = kf_broker::capture_log();
    for _ in 0..300 {
        t.publish(4096, 4096); // 64 MiB of pixels in a 64 KiB backing
        t.frame();
    }
    assert_eq!(t.relay.counters().refused, 300);
    let said = log
        .lines()
        .iter()
        .filter(|l| l.contains("REFUSED frame slot"))
        .count();
    assert_eq!(said, 5, "frames 1-4 and 256: {:?}", log.lines());
}

/// Stop unwatches before it closes, and leaves no timer.
#[test]
fn stop_unwatches_then_closes() {
    let mut t = T::new(false);
    t.up(0);
    t.relay.stop(&mut t.host);
    assert_eq!(t.host.watches.last(), Some(&(77, false, false)));
    assert_eq!(t.wire.borrow().open, 0);
    assert_eq!(t.host.timer, None);
    t.tick(10_000);
    assert_eq!(
        t.wire.borrow().connects,
        1,
        "a stopped relay never reconnects"
    );
}

// ── the withdrawal on the owed and replay paths (the third review, 2026-10-03) ─────────────

/// Lines that say the connection went down or an attempt failed.
fn lost_lines(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|l| {
            l.contains("gone for now") || l.contains("attempt failed") || l.contains("failed:")
        })
        .cloned()
        .collect()
}

/// ★ The third review's major: once the worker WITHDREW the ring, an owed frame — its ATTACH
/// owed, or only its COMMIT — is refused and forgotten. The connection stays ACTIVE (no "lost"
/// or "failed" line, input still flows), and nothing more of the frame goes: no ATTACH, no
/// COMMIT, `sent` unchanged. ⊘ Before the fix, `attach` turned the unfit slot into
/// `Sent::Failed`, and `flush_owed` dropped the connection over it.
#[test]
fn an_owed_frame_after_the_withdrawal_is_refused_and_the_connection_stays_up() {
    // owed ATTACH (the WINDOW found the socket full), then via writability and via the timer
    for by_timer in [false, true] {
        let mut t = T::new(false);
        t.up(0);
        t.wire.borrow_mut().budget = Some(0);
        let j = t.publish(64, 32);
        t.frame();
        assert_eq!(t.relay.counters().dropped, 1, "the ATTACH is owed");
        assert_eq!(t.ring.held_mask(), 1 << j);
        let log = kf_broker::capture_log();
        t.ring.withdraw_all(); // the worker, on its own thread
        t.wire.borrow_mut().budget = None;
        if by_timer {
            t.tick(kf_broker::conn::OWED_MS);
        } else {
            t.writable();
        }
        assert!(t.relay.active(), "the withdrawal tore the connection down");
        assert!(
            t.sent().is_empty(),
            "the owed frame was sent: {:?}",
            t.types()
        );
        let c = t.relay.counters();
        assert_eq!((c.sent, c.refused, c.recovered), (0, 1, 0));
        assert_eq!(t.ring.held_mask(), 0, "its slot came back");
        assert!(lost_lines(&log.lines()).is_empty(), "{:?}", log.lines());
        assert!(
            log.lines()
                .iter()
                .any(|l| l.contains("REFUSED frame slot") && l.contains("withdrawn")),
            "{:?}",
            log.lines()
        );
        // input still flows, and no owed write is watched for
        t.pkt(EV_KEY, 30, 1, 0, 0);
        assert_eq!(
            t.read(),
            vec![Input::Key {
                code: 30,
                down: true
            }]
        );
        assert_eq!(t.host.watches.last().map(|w| w.2), Some(false));
        t.tick(1000);
        assert!(t.sent().is_empty() && t.relay.active());
    }
    // owed COMMIT (WINDOW and ATTACH went): the COMMIT is dropped, never sent
    let mut t = T::new(false);
    t.up(0);
    t.wire.borrow_mut().budget = Some(2);
    let j = t.publish(64, 32);
    t.frame();
    assert_eq!(t.types(), vec![CMD_WINDOW, CMD_ATTACH]);
    assert_eq!(t.relay.counters().uncommitted, 1, "the COMMIT is owed");
    t.clear();
    let log = kf_broker::capture_log();
    t.ring.withdraw_all();
    t.wire.borrow_mut().budget = None;
    t.writable();
    assert!(t.relay.active());
    assert!(
        t.sent().is_empty(),
        "an owed COMMIT went after the withdrawal"
    );
    let c = t.relay.counters();
    assert_eq!((c.sent, c.refused), (0, 1));
    assert_eq!(t.ring.held_mask() & (1 << j), 0);
    assert!(lost_lines(&log.lines()).is_empty(), "{:?}", log.lines());
}

/// ★ The cross-thread race the review named: the worker withdraws the ring BETWEEN two of the
/// relay's sends for one frame (after the relay's `fits` passed). The live path, after the
/// WINDOW and after the ATTACH; and the replay, after its ATTACH. Each time the frame is
/// refused, the connection stays up (the replay reaches CAPS), and no COMMIT follows.
#[test]
fn a_withdrawal_between_two_sends_refuses_the_frame_and_keeps_the_connection() {
    for after in [CMD_WINDOW, CMD_ATTACH] {
        let mut t = T::new(false);
        t.up(0);
        let ring = t.ring.clone();
        t.wire.borrow_mut().after_send = Some((after, Box::new(move || ring.withdraw_all())));
        let log = kf_broker::capture_log();
        t.publish(64, 32);
        t.frame();
        let want = if after == CMD_WINDOW {
            vec![CMD_WINDOW]
        } else {
            vec![CMD_WINDOW, CMD_ATTACH]
        };
        assert_eq!(t.types(), want, "withdrawn after {after}");
        assert!(t.relay.active());
        let c = t.relay.counters();
        assert_eq!((c.sent, c.refused), (0, 1), "withdrawn after {after}");
        assert_eq!(t.ring.held_mask(), 0);
        assert!(lost_lines(&log.lines()).is_empty(), "{:?}", log.lines());
    }
    // the replay: a held frame is requeued; the ring is withdrawn after the replay's ATTACH
    let mut t = T::new(false);
    t.up(0);
    let k = t.publish(64, 32);
    t.frame();
    t.wire.borrow_mut().eof = true;
    t.read();
    assert_eq!(t.ring.broker_ready(), Some(k));
    t.wire.borrow_mut().eof = false;
    t.clear();
    let ring = t.ring.clone();
    t.wire.borrow_mut().after_send = Some((CMD_ATTACH, Box::new(move || ring.withdraw_all())));
    let log = kf_broker::capture_log();
    t.now = t.host.timer.unwrap();
    t.pkt(EV_HELLO, 0, 0, 2, CAP_DMABUF);
    let now = t.now;
    t.relay.on_timer(now, &mut t.host);
    t.read();
    assert!(t.relay.active(), "the replay must reach CAPS");
    assert_eq!(t.types(), vec![CMD_WINDOW, CMD_ATTACH, CMD_CAPS]);
    let c = t.relay.counters();
    assert_eq!((c.refused, c.attempts_failed), (1, 0));
    assert!(lost_lines(&log.lines()).is_empty(), "{:?}", log.lines());
}
