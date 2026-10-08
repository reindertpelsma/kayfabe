// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ The relay's connection machine against a scripted link and a deterministic clock — every
//! invariant of `docs/design/V3_DISPLAY.md` §8.1 and each fix over nvkvm-pv, one test each.

use kf_broker::wire::{
    CAP_DMABUF, CAP_FOCUS_EVENTS, CAP_MODIFIERS, CAP_RELEASE, CMD_ATTACH, CMD_CAPS, CMD_COMMIT,
    CMD_F_SHM, CMD_QUERY_FORMAT, CMD_SIZE, CMD_WINDOW, Cmd, EV_ABS, EV_BTN, EV_CLOSE, EV_FORMAT,
    EV_FRAME, EV_GRAB, EV_HELLO, EV_KEY, EV_REL, EV_RELEASE, EV_SURFACE, EV_WHEEL, FOURCC_AR24,
    FOURCC_XR24, MOD_INVALID, MOD_LINEAR, PKT_SIZE, Pkt,
};
use kf_broker::{
    FrameGeom, FrameRing, Host, Input, Kind, Link, Pointer, Recv, Relay, RelayConfig, Sent,
    SlotFds, VramFds, VramGeom,
};
use kf_linux_raw::{SharedRam, fd_inode};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::os::fd::{BorrowedFd, OwnedFd};
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
        T::with_dmabuf(|| {
            dmabuf.then(|| {
                SharedRam::create_named(c"kfb-test-dmabuf", 64 * 1024)
                    .unwrap()
                    .dup_for_export()
                    .unwrap()
            })
        })
    }

    /// As [`T::new`], with each slot's dma-buf descriptor made by `dma`.
    fn with_dmabuf(dma: impl Fn() -> Option<OwnedFd>) -> T {
        let ring = Arc::new(FrameRing::new(kf_broker::slots::BROKER_SLOTS, true));
        for j in 0..ring.slots() {
            let mem = SharedRam::create_named(c"kfb-test-frame", 64 * 1024).unwrap();
            ring.install(j, SlotFds::new(mem, dma()).unwrap()).unwrap();
        }
        T::over(ring)
    }

    /// A relay over `ring` as it is (slots installed or not).
    fn over(ring: Arc<FrameRing>) -> T {
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
/// wheel by sign, the horizontal one too (since §8.20); GRAB and CLOSE handed over.
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
    t.pkt(EV_SURFACE, 1600, 900, 59_940, 0); // the same size and rate again: no second hint
    // ★ §8.16 (`display-max-fps`): the same size at another refresh IS a change (the guest's
    // preferred rate follows the host's monitor) — before, the size alone was compared
    t.pkt(EV_SURFACE, 1600, 900, 30_000, 0);
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
                down: false,
                to: Pointer::Absolute
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
            Input::Wheel {
                dx: 0,
                dy: -1,
                to: Pointer::Absolute
            },
            // ★ 2026-10-08 (§8.20): the horizontal detent is handed on by sign (before: dropped
            // here); a VMM without a horizontal wheel ignores it at its sink
            Input::Wheel {
                dx: 1,
                dy: 0,
                to: Pointer::Absolute
            },
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
                w: 1600,
                h: 900,
                mhz: 30_000
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

// ── rung 0: the GPU copy (V3_DISPLAY.md §8.11, OWNER_RULINGS.md §L) ─────────────────────────

/// The GPU-copy modifier the fixture's device answers (`0x0300000000606014`, Turing+ XR24 h=4).
const BL: u64 = 0x0300_0000_0060_6014;
/// The fixture GPU's DRM nodes.
const NODES: [(u32, u32); 2] = [(226, 1), (226, 129)];

impl T {
    /// As [`T::new`], plus a VRAM backing per slot (a memfd standing in for the dma-buf: the ring
    /// takes any descriptor), the device's GPU-copy modifier and its DRM nodes.
    fn with_vram() -> T {
        T::vram_over(T::new(false))
    }

    /// `t` plus [`T::with_vram`]'s VRAM backings, modifier and nodes.
    fn vram_over(t: T) -> T {
        for j in 0..t.ring.slots() {
            let fd = SharedRam::create_named(c"kfb-test-vram", 64 * 1024)
                .unwrap()
                .dup_for_export()
                .unwrap();
            t.ring
                .install_vram(j, VramFds::new(fd, 64 * 1024).unwrap())
                .unwrap();
        }
        t.ring.set_vram_modifier(BL);
        t.ring.set_gpu_nodes(NODES.to_vec());
        t
    }

    /// Publish a `w`x`h` frame whose host backing is fresh when `host`, and whose VRAM backing is
    /// fresh (packed at the slot geometry) when `vram`.
    fn publish_kinds(&mut self, w: u32, h: u32, host: bool, vram: bool) -> usize {
        let t = self.publish_unannounced(w, h);
        let gpr = (w * 4).div_ceil(64);
        let vg = VramGeom {
            stride: gpr * 64,
            extent: u64::from(h.div_ceil(128)) * u64::from(gpr) * 8192,
        };
        self.ring.describe_backings(t, host, vram.then_some(vg));
        self.ring.publish(t);
        t
    }

    fn publish_unannounced(&mut self, w: u32, h: u32) -> usize {
        let t = self.ring.fill_target(None).expect("a fill target");
        self.serial += 1;
        self.ring.describe(
            t,
            FrameGeom {
                width: w,
                height: h,
                stride: w * 4,
                fourcc: FOURCC_XR24,
                serial: self.serial,
            },
        );
        t
    }

    fn vram_id(&self, j: usize) -> u64 {
        self.ring.vram(j).unwrap().id()
    }

    /// The broker's answer for the block-linear pair.
    fn bl_verdict(&self, yes: bool) {
        self.pkt(
            EV_FORMAT,
            i32::from(yes),
            FOURCC_XR24 as i32,
            BL as u32,
            (BL >> 32) as u32,
        );
    }

    /// The broker is done with the native frame in `slot`.
    fn release_vram(&self, slot: usize) {
        let id = self.vram_id(slot);
        self.pkt(EV_RELEASE, 0, 0, id as u32, (id >> 32) as u32);
    }
}

const NATIVE_CAPS: u32 = CAP_MODIFIERS | CAP_RELEASE;

/// ★ Rung 0 needs an EXPLICIT yes to the block-linear pair, asked right after HELLO (with
/// `display-broker-vram=auto` the slots are provisioned only then), plus `CAP_MODIFIERS` and
/// `CAP_RELEASE`. Then a frame whose VRAM backing is fresh goes as the VRAM dma-buf, with the
/// block-linear modifier and stride, and the worker is told to pack (`want_vram`).
#[test]
fn the_gpu_copy_goes_only_after_an_explicit_yes() {
    let mut t = T::with_vram();
    t.up_keep(NATIVE_CAPS);
    let q: Vec<_> = t
        .sent()
        .into_iter()
        .filter(|(c, _)| c.ty == CMD_QUERY_FORMAT)
        .map(|(c, _)| (c.fourcc, c.modifier))
        .collect();
    assert_eq!(
        q,
        vec![(FOURCC_XR24, BL)],
        "the block-linear pair, at HELLO"
    );
    t.clear();
    assert!(!t.ring.want_vram(), "no verdict yet");
    // before the answer: the host rung, never the VRAM backing
    let j = t.publish_kinds(64, 32, true, true);
    t.frame();
    let att = t
        .sent()
        .into_iter()
        .find(|(c, _)| c.ty == CMD_ATTACH)
        .unwrap();
    assert_eq!(att.0.flags, CMD_F_SHM);
    assert_eq!(att.1, Some(t.memfd_id(j)));
    t.clear();
    t.bl_verdict(true);
    t.read();
    assert!(t.ring.want_vram(), "the worker packs now");
    t.pkt(EV_FRAME, 0, 0, 0, 0);
    t.read();
    let k = t.publish_kinds(64, 32, true, true);
    t.frame();
    let att = t
        .sent()
        .into_iter()
        .find(|(c, _)| c.ty == CMD_ATTACH)
        .unwrap();
    assert_eq!(
        (att.0.modifier, att.0.stride, att.0.flags),
        (BL, 256, 0),
        "the block-linear modifier and stride"
    );
    assert_eq!(att.1, Some(t.vram_id(k)), "the VRAM dma-buf went");
    assert_eq!(t.relay.counters().native, 1);
    // without CAP_RELEASE (no real RELEASE, so no detector) or CAP_MODIFIERS: never rung 0
    for caps in [CAP_MODIFIERS, CAP_RELEASE] {
        let mut t = T::with_vram();
        t.up(caps);
        t.bl_verdict(true);
        t.read();
        assert!(!t.ring.want_vram(), "caps {caps:#x}");
        t.publish_kinds(64, 32, true, true);
        t.frame();
        assert!(
            t.sent().iter().all(|(c, _)| c.modifier != BL),
            "caps {caps:#x}: a VRAM frame went"
        );
    }
}

/// A device without a GPU-copy modifier never asks the pair and never sends rung 0.
#[test]
fn a_device_without_the_rung_never_asks_or_sends_it() {
    let mut t = T::with_vram();
    t.ring.set_vram_modifier(0);
    t.up_keep(NATIVE_CAPS);
    assert!(t.types().iter().all(|ty| *ty != CMD_QUERY_FORMAT));
    t.clear();
    t.publish_kinds(64, 32, true, true);
    t.frame();
    assert!(t.sent().iter().all(|(c, _)| c.modifier != BL));
    assert_eq!(t.relay.counters().native, 0);
}

/// ★ A later "no" for the block-linear pair (Wayland's failed probe; X11 once the broker sends
/// it) stops rung 0, reclaims the frames attached under it, and the next frame goes on a host
/// rung.
#[test]
fn a_later_no_for_the_block_linear_pair_falls_back_and_reclaims() {
    let mut t = T::with_vram();
    t.up(NATIVE_CAPS);
    t.bl_verdict(true);
    t.read();
    let j = t.publish_kinds(64, 32, true, true);
    t.frame();
    assert_eq!(t.relay.counters().native, 1);
    assert_eq!(t.ring.held_mask(), 1 << j);
    t.clear();
    t.bl_verdict(false);
    t.read();
    assert!(!t.ring.want_vram());
    assert_eq!(t.ring.held_mask(), 0, "the native frame was reclaimed");
    let k = t.publish_kinds(64, 32, true, true);
    t.frame();
    let att = t
        .sent()
        .into_iter()
        .find(|(c, _)| c.ty == CMD_ATTACH)
        .unwrap();
    assert_eq!(att.1, Some(t.memfd_id(k)), "back on the host rung");
}

/// ★ The acknowledgement detector — the box experiment E2(d)'s logic without a GPU: three native
/// commits and a second with no RELEASE trip a 5 s back-off (frames go on a host rung, the worker
/// stops packing), rung 0 is tried again after it, a second trip doubles it, and a native RELEASE
/// confirms the rung and clears the back-off.
#[test]
fn the_detector_backs_off_retries_and_clears_on_a_release() {
    let mut t = T::with_vram();
    t.up(NATIVE_CAPS);
    t.bl_verdict(true);
    t.read();
    let log = kf_broker::capture_log();
    let native_frame = |t: &mut T| {
        t.publish_kinds(64, 32, true, true);
        t.frame();
        t.pkt(EV_FRAME, 0, 0, 0, 0); // pacing, but no RELEASE: the compositor never imported it
        t.read();
    };
    // an unimported frame is never released, so the cap of 2 and the 1 s reclaim pace the
    // commits: the third goes once the first is reclaimed, and the trip follows
    let t0 = t.now;
    while t.relay.counters().native_trips == 0 {
        assert!(t.now - t0 <= 3000, "no trip within 3 s");
        native_frame(&mut t);
        t.tick(100);
    }
    assert!(t.relay.counters().native >= 3);
    assert!(!t.ring.want_vram(), "the worker stops packing");
    assert!(
        log.lines()
            .iter()
            .any(|l| l.contains("not acknowledged") && l.contains("5 s")),
        "{:?}",
        log.lines()
    );
    t.clear();
    t.publish_kinds(64, 32, true, true);
    t.frame();
    assert!(
        t.sent().iter().all(|(c, _)| c.modifier != BL),
        "backing off: a host rung"
    );
    // after the back-off, rung 0 again; unconfirmed again ⇒ a 10 s back-off
    t.tick(5000);
    assert!(t.ring.want_vram(), "tried again after 5 s");
    let t1 = t.now;
    while t.relay.counters().native_trips == 1 {
        assert!(t.now - t1 <= 3000, "no second trip within 3 s");
        native_frame(&mut t);
        t.tick(100);
    }
    assert_eq!(t.relay.counters().native_trips, 2);
    assert!(log.lines().iter().any(|l| l.contains("10 s")));
    t.tick(10_000);
    assert!(t.ring.want_vram());
    // a native RELEASE confirms the rung: no more trips however long it goes unreleased
    t.pkt(EV_FRAME, 0, 0, 0, 0);
    t.read();
    let j = t.publish_kinds(64, 32, true, true);
    t.frame();
    t.release_vram(j);
    t.read();
    assert!(
        log.lines()
            .iter()
            .any(|l| l.contains("imported a GPU-copy frame"))
    );
    for _ in 0..6 {
        native_frame(&mut t);
        t.tick(600);
    }
    assert_eq!(t.relay.counters().native_trips, 2, "confirmed: never again");
    assert!(t.ring.want_vram());
}

/// ★ `EV_DEVICE` (nvkvm-pv's header: `x` = `DEVICE_F_*`, `w0`:`w1` = major:minor): a compositor
/// on another device's RENDER node gets no GPU-copy frame on this connection, even after a yes;
/// one on this GPU's primary or render node does; "does not know" (`x` = 0) leaves the yes and
/// the detector to decide; an unresolved node (KNOWN without RENDER) decides only when it is this
/// GPU's. A broker advertising `CAP_DEVICE` gets no GPU-copy frame before its `EV_DEVICE`. A
/// later, different device (sent unsolicited) moves the decision.
#[test]
fn a_compositor_on_another_gpu_gets_no_gpu_copy() {
    use kf_broker::wire::{CAP_DEVICE, DEVICE_F_KNOWN, DEVICE_F_RENDER, EV_DEVICE};
    let kr = DEVICE_F_KNOWN | DEVICE_F_RENDER;
    let mut t = T::with_vram();
    t.up(NATIVE_CAPS | CAP_DEVICE);
    t.bl_verdict(true);
    t.read();
    assert!(!t.ring.want_vram(), "CAP_DEVICE: wait for EV_DEVICE");
    let log = kf_broker::capture_log();
    t.pkt(EV_DEVICE, kr, 0, 226, 130);
    t.read();
    assert!(!t.ring.want_vram());
    assert!(
        log.lines()
            .iter()
            .any(|l| l.contains("226:130") && l.contains("not this GPU")),
        "{:?}",
        log.lines()
    );
    t.publish_kinds(64, 32, true, true);
    t.frame();
    assert!(t.sent().iter().all(|(c, _)| c.modifier != BL));
    // the display server moves to this GPU: an unsolicited EV_DEVICE says so
    t.pkt(EV_DEVICE, kr, 0, 226, 129);
    t.read();
    assert!(
        t.ring.want_vram(),
        "a later, different device moves the decision"
    );
    for (flags, (major, minor)) in [(kr, NODES[1]), (DEVICE_F_KNOWN, NODES[0])] {
        let mut t = T::with_vram();
        t.up(NATIVE_CAPS | CAP_DEVICE);
        t.pkt(EV_DEVICE, flags, 0, major, minor);
        t.bl_verdict(true);
        t.read();
        assert!(t.ring.want_vram(), "{major}:{minor} is this GPU");
    }
    // "does not know", and an unresolved node that is not ours: the yes and the detector decide
    for (flags, dev) in [(0, (0, 0)), (DEVICE_F_KNOWN, (226, 2))] {
        let mut t = T::with_vram();
        t.up(NATIVE_CAPS | CAP_DEVICE);
        t.pkt(EV_DEVICE, flags, 0, dev.0, dev.1);
        t.bl_verdict(true);
        t.read();
        assert!(t.ring.want_vram(), "flags {flags:#x} {dev:?}");
    }
    // an older broker (no CAP_DEVICE) never sends it: the yes decides
    let mut t = T::with_vram();
    t.up(NATIVE_CAPS);
    t.bl_verdict(true);
    t.read();
    assert!(t.ring.want_vram());
}

/// ★ Freshness: a frame only in VRAM is never sent on a host rung (it would show stale pixels)
/// and is refused while rung 0 is not allowed; a frame only in host memory is never sent from
/// the stale VRAM backing.
#[test]
fn a_stale_backing_is_never_sent() {
    let mut t = T::with_vram();
    t.up(NATIVE_CAPS);
    // no verdict yet: a VRAM-only frame has nothing to go on
    t.publish_kinds(64, 32, false, true);
    t.frame();
    assert!(t.sent().iter().all(|(c, _)| c.ty != CMD_ATTACH));
    assert_eq!(t.relay.counters().refused, 1);
    t.bl_verdict(true);
    t.read();
    t.clear();
    // rung 0 allowed, but this frame was not packed: the host backing goes
    let j = t.publish_kinds(64, 32, true, false);
    t.frame();
    let att = t
        .sent()
        .into_iter()
        .find(|(c, _)| c.ty == CMD_ATTACH)
        .unwrap();
    assert_eq!(att.1, Some(t.memfd_id(j)));
}

/// ★ Per-kind withdrawal in the relay: with the host kind withdrawn the GPU copy goes on; with
/// the VRAM kind withdrawn the host rungs go on — neither failure takes the other rung down.
#[test]
fn a_withdrawn_kind_leaves_the_other_rung_working() {
    let mut t = T::with_vram();
    t.up(NATIVE_CAPS);
    t.bl_verdict(true);
    t.read();
    t.ring.withdraw(Kind::Host);
    let j = t.publish_kinds(64, 32, true, true);
    t.frame();
    let att = t
        .sent()
        .into_iter()
        .find(|(c, _)| c.ty == CMD_ATTACH)
        .unwrap();
    assert_eq!(att.1, Some(t.vram_id(j)));
    t.clear();
    let mut t = T::with_vram();
    t.up(NATIVE_CAPS);
    t.bl_verdict(true);
    t.read();
    t.ring.withdraw(Kind::Vram);
    assert!(
        !t.ring.want_vram() || t.ring.withdrawn(Kind::Vram),
        "the worker sees the withdrawal itself"
    );
    let j = t.publish_kinds(64, 32, true, true);
    t.frame();
    let att = t
        .sent()
        .into_iter()
        .find(|(c, _)| c.ty == CMD_ATTACH)
        .unwrap();
    assert_eq!(att.1, Some(t.memfd_id(j)));
}

/// A VRAM backing too small for the frame's block-linear extent is never sent (the host rung
/// takes the frame), and a stride outside the broker's bounds is refused before the wire.
#[test]
fn a_vram_frame_outside_its_object_or_the_brokers_bounds_is_not_sent() {
    let mut t = T::with_vram();
    t.up(NATIVE_CAPS);
    t.bl_verdict(true);
    t.read();
    // 4096 rows of 64 px: extent 32 block rows * 4 GOBs * 8 KiB = 1 MiB > the 64 KiB backing
    let t2 = t.publish_unannounced(64, 4096);
    t.ring.describe_backings(
        t2,
        false,
        Some(VramGeom {
            stride: 256,
            extent: 1 << 20,
        }),
    );
    t.ring.publish(t2);
    t.frame();
    assert!(t.sent().iter().all(|(c, _)| c.ty != CMD_ATTACH));
    assert_eq!(t.relay.counters().refused, 1);
    // a stride the broker would refuse (> 8w + 4096)
    let t3 = t.publish_unannounced(64, 32);
    t.ring.describe_backings(
        t3,
        false,
        Some(VramGeom {
            stride: 8192,
            extent: 32768,
        }),
    );
    t.ring.publish(t3);
    t.pkt(EV_FRAME, 0, 0, 0, 0);
    t.read();
    t.frame();
    assert!(t.sent().iter().all(|(c, _)| c.ty != CMD_ATTACH));
    assert_eq!(t.relay.counters().refused, 2);
}

// ── nvkvm-pv `badf2d7`: what the broker now does that the relay must handle ──────────────────

impl T {
    /// The first ATTACH sent since the last [`T::clear`].
    fn attach_sent(&self) -> (Cmd, Option<u64>) {
        self.sent()
            .into_iter()
            .find(|(c, _)| c.ty == CMD_ATTACH)
            .expect("an ATTACH")
    }

    /// The broker is done with the dma-buf frame in `slot`.
    fn release_dmabuf(&self, slot: usize) {
        let id = self.dmabuf_id(slot);
        self.pkt(EV_RELEASE, 0, 0, id as u32, (id >> 32) as u32);
    }

    /// Publish a frame and run the clock (100 ms steps, at most 2 s) until its ATTACH went — two
    /// unreleased frames hold the cap until the reclaim gives one back. Its ATTACH, after a
    /// [`T::clear`].
    fn next_attach(&mut self) -> (usize, Cmd, Option<u64>) {
        self.clear();
        let j = self.publish(64, 32);
        self.frame();
        for _ in 0..20 {
            if self.types().contains(&CMD_ATTACH) {
                break;
            }
            self.tick(100);
        }
        let (c, id) = self.attach_sent();
        self.pkt(EV_FRAME, 0, 0, 0, 0);
        self.read();
        (j, c, id)
    }

    /// One dma-buf frame, committed and paced, never released (the broker dropped it).
    fn unreleased_frame(&mut self) -> usize {
        let j = self.publish(64, 32);
        self.frame();
        self.pkt(EV_FRAME, 0, 0, 0, 0);
        self.read();
        j
    }

    /// Lose the connection and come back with `caps`; the replay is kept.
    fn reconnect(&mut self, caps: u32) {
        self.wire.borrow_mut().eof = true;
        self.read();
        assert!(!self.relay.connected());
        self.wire.borrow_mut().eof = false;
        self.clear();
        self.now = self.host.timer.expect("a retry");
        self.pkt(EV_HELLO, 0, 0, 2, caps | CAP_DMABUF);
        let now = self.now;
        self.relay.on_timer(now, &mut self.host);
        self.read();
        assert!(self.relay.active());
    }
}

/// ★ `badf2d7` (1): the broker volunteers `EV_FORMAT x=0` whenever an ATTACH is dropped at its
/// format gate — possibly for a pair this connection never asked about. The relay RECORDS it: no
/// question about the pair, and the frame takes another rung. Known-positive: the same script with
/// the "no" withheld asks and goes LINEAR. And an unasked YES is never an upgrade.
#[test]
fn an_unasked_no_is_recorded_and_its_pair_is_not_sent() {
    for told in [false, true] {
        let mut t = T::new(true);
        t.up(CAP_MODIFIERS | CAP_RELEASE);
        if told {
            t.pkt(EV_FORMAT, 0, FOURCC_XR24 as i32, 0, 0); // (XR24, LINEAR), never asked
            t.read();
        }
        let j = t.publish(64, 32);
        t.frame();
        let att = t.attach_sent();
        if told {
            assert!(
                !t.types().contains(&CMD_QUERY_FORMAT),
                "told: no question about a recorded no"
            );
            assert_eq!((att.0.flags, att.1), (CMD_F_SHM, Some(t.memfd_id(j))));
            assert_eq!(t.relay.counters().formats_unasked, 1);
        } else {
            assert_eq!(t.types()[0], CMD_QUERY_FORMAT, "untold: asked");
            assert_eq!(
                (att.0.flags, att.0.modifier, att.1),
                (0, MOD_LINEAR, Some(t.dmabuf_id(j)))
            );
        }
    }
    // a volunteered YES for the implicit modifier: still asked, shared memory meanwhile
    let mut t = T::new(true);
    t.up(CAP_RELEASE);
    t.pkt(
        EV_FORMAT,
        1,
        FOURCC_XR24 as i32,
        MOD_INVALID as u32,
        (MOD_INVALID >> 32) as u32,
    );
    t.read();
    t.publish(64, 32);
    t.frame();
    assert_eq!(
        t.types(),
        vec![CMD_QUERY_FORMAT, CMD_WINDOW, CMD_ATTACH, CMD_COMMIT]
    );
    assert_eq!(
        t.sent()[2].0.flags,
        CMD_F_SHM,
        "an unasked yes is no answer"
    );
}

/// ★ `badf2d7` (1), the bound: the table holds 16 verdicts, and however many "no"s the broker
/// volunteers for pairs the relay does not send, the "no" for a pair it DOES send stays — the
/// broker tells each pair once per connection, so a lost one is a black window. Known-positive:
/// the 4-row recycle-the-oldest table this replaced (`relay.c:92`) lost it after four.
#[test]
fn volunteered_noes_never_push_out_a_no_for_a_pair_the_relay_sends() {
    let mut t = T::new(true);
    t.up(CAP_MODIFIERS | CAP_RELEASE);
    t.publish(64, 32);
    t.frame(); // asks (XR24, LINEAR)
    t.pkt(EV_FORMAT, 0, FOURCC_XR24 as i32, 0, 0);
    for m in 0..40u32 {
        t.pkt(EV_FORMAT, 0, FOURCC_AR24 as i32, 0x100 + m, 0x0300_0000);
    }
    t.read();
    assert_eq!(t.relay.counters().formats_unasked, 40);
    t.pkt(EV_FRAME, 0, 0, 0, 0);
    t.read();
    t.clear();
    let k = t.publish(64, 32);
    t.frame();
    assert!(
        !t.types().contains(&CMD_QUERY_FORMAT),
        "the relay's own no is still known"
    );
    assert_eq!(t.attach_sent(), (t.attach_sent().0, Some(t.memfd_id(k))));
    assert_eq!(t.attach_sent().0.flags, CMD_F_SHM);
}

/// ★ `badf2d7` (2): on X11 one refused import arrives as TWO `x=0` — XR24 and AR24, the same
/// modifier (DRI3 imports the two identically). Both are kept, and the frames held under the
/// refused pair come back ONCE; an AR24 frame then asks nothing and goes as shared memory.
/// Known-positive: with the twin's "no" withheld the AR24 frame asks and goes LINEAR.
#[test]
fn an_x11_refusal_names_both_alpha_twins_and_both_are_kept() {
    for twin in [false, true] {
        let mut t = T::new(true);
        t.up(CAP_MODIFIERS | CAP_RELEASE);
        let j = t.publish(64, 32);
        t.frame();
        assert_eq!(t.ring.held_mask(), 1 << j);
        t.pkt(EV_FORMAT, 0, FOURCC_XR24 as i32, 0, 0);
        if twin {
            t.pkt(EV_FORMAT, 0, FOURCC_AR24 as i32, 0, 0);
        }
        t.read();
        assert_eq!(t.ring.held_mask(), 0, "the refused frame came back");
        assert_eq!(t.relay.counters().reclaims, 1, "once, twin or not");
        t.clear();
        let k = t.publish_fourcc(64, 32, FOURCC_AR24);
        t.frame();
        let att = t.attach_sent();
        if twin {
            assert!(!t.types().contains(&CMD_QUERY_FORMAT));
            assert_eq!((att.0.flags, att.1), (CMD_F_SHM, Some(t.memfd_id(k))));
        } else {
            assert_eq!(t.types()[0], CMD_QUERY_FORMAT);
            assert_eq!((att.0.flags, att.0.modifier), (0, MOD_LINEAR));
        }
    }
}

/// ★ `badf2d7` (3): refusals are CONNECTION state on both backends — a refusal is caused by a
/// buffer the guest chose, and on a `--persist` broker it must not decide the next VM's present
/// path — so the relay carries none across a reconnect either: not an unasked "no", not the
/// block-linear pair's, and not the dma-buf detector's back-off. Known-positive: on the SAME
/// connection each of them holds (asserted before the reconnect).
#[test]
fn no_refusal_or_back_off_outlives_its_connection() {
    let mut t = T::vram_over(T::new(true));
    t.up(NATIVE_CAPS);
    t.bl_verdict(true);
    t.read();
    assert!(t.ring.want_vram());
    // the display takes the block-linear yes back, and volunteers a no for (XR24, LINEAR)
    t.bl_verdict(false);
    t.pkt(EV_FORMAT, 0, FOURCC_XR24 as i32, 0, 0);
    t.read();
    assert!(!t.ring.want_vram(), "this connection: no GPU copy");
    t.clear();
    let k = t.publish_kinds(64, 32, true, true);
    t.frame();
    assert_eq!(
        t.attach_sent().1,
        Some(t.memfd_id(k)),
        "this connection: F_SHM"
    );
    t.reconnect(NATIVE_CAPS);
    let q: Vec<_> = t
        .sent()
        .iter()
        .filter(|(c, _)| c.ty == CMD_QUERY_FORMAT)
        .map(|(c, _)| (c.fourcc, c.modifier))
        .collect();
    assert!(q.contains(&(FOURCC_XR24, BL)), "asked again: {q:?}");
    t.bl_verdict(true);
    t.pkt(EV_FRAME, 0, 0, 0, 0);
    t.read();
    assert!(t.ring.want_vram(), "the new connection's yes counts");
    t.clear();
    let n = t.publish_kinds(64, 32, true, true);
    t.frame();
    assert_eq!(t.attach_sent().1, Some(t.vram_id(n)), "the GPU copy again");
    t.release_vram(n);
    t.read();
    t.clear();
    let h = t.publish_kinds(64, 32, true, false);
    t.frame();
    assert_eq!(
        t.attach_sent().1,
        Some(t.dmabuf_id(h)),
        "the old unasked no was forgotten: LINEAR"
    );
    // the detector's back-off: tripped on one connection, gone on the next
    let mut t = T::new(true);
    t.up(CAP_MODIFIERS | CAP_RELEASE);
    let t0 = t.now;
    while t.relay.counters().dmabuf_trips == 0 {
        assert!(t.now - t0 <= 3000, "no trip within 3 s");
        t.unreleased_frame();
        t.tick(100);
    }
    let (k, _, id) = t.next_attach();
    assert_eq!(id, Some(t.memfd_id(k)), "backing off: F_SHM");
    t.reconnect(CAP_MODIFIERS | CAP_RELEASE);
    t.pkt(EV_FRAME, 0, 0, 0, 0);
    t.read();
    t.clear();
    let j = t.publish(64, 32);
    t.frame();
    assert_eq!(
        t.attach_sent().1,
        Some(t.dmabuf_id(j)),
        "a new connection: LINEAR"
    );
}

/// ★ `badf2d7` (4): the broker needs a readable `/proc/self/fdinfo` to accept a dma-buf frame, and
/// without one drops every dma-buf ATTACH with NO word on the wire (it is not the format gate, so
/// no `EV_FORMAT`; the connection lives) — LINEAR frames commit and never come back. The host
/// dma-buf detector backs them off to shared memory (5 s, then 10 s), tries LINEAR again, and a
/// LINEAR RELEASE confirms the rung for the connection. Known-negatives: with RELEASEs coming back
/// nothing trips; without `CAP_RELEASE` nothing is counted.
#[test]
fn unacknowledged_dma_buf_frames_back_off_to_shared_memory_and_retry() {
    let mut t = T::new(true);
    t.up(CAP_MODIFIERS | CAP_RELEASE);
    let log = kf_broker::capture_log();
    let t0 = t.now;
    while t.relay.counters().dmabuf_trips == 0 {
        assert!(t.now - t0 <= 3000, "no trip within 3 s");
        t.unreleased_frame();
        t.tick(100);
    }
    assert!(
        log.lines()
            .iter()
            .any(|l| l.contains("/proc/self/fdinfo") && l.contains("5 s")),
        "{:?}",
        log.lines()
    );
    let (k, c, id) = t.next_attach();
    assert_eq!(
        (c.flags, id),
        (CMD_F_SHM, Some(t.memfd_id(k))),
        "backing off: shared memory"
    );
    t.tick(5000);
    let (j, c, id) = t.next_attach();
    assert_eq!(
        (c.modifier, id),
        (MOD_LINEAR, Some(t.dmabuf_id(j))),
        "LINEAR again after 5 s"
    );
    t.release_dmabuf(j);
    t.read();
    for _ in 0..30 {
        t.unreleased_frame();
        t.tick(100);
    }
    assert_eq!(
        t.relay.counters().dmabuf_trips,
        1,
        "acknowledged: never again"
    );
    // known-negative: released frames never trip
    let mut t = T::new(true);
    t.up(CAP_MODIFIERS | CAP_RELEASE);
    for _ in 0..30 {
        let j = t.unreleased_frame();
        t.release_dmabuf(j);
        t.read();
        t.tick(100);
    }
    assert_eq!(t.relay.counters().dmabuf_trips, 0);
    // no CAP_RELEASE: no RELEASE is promised, so nothing is counted
    let mut t = T::new(true);
    t.up(CAP_MODIFIERS);
    for _ in 0..30 {
        t.unreleased_frame();
        t.tick(100);
    }
    assert_eq!(t.relay.counters().dmabuf_trips, 0);
}

/// ★ `badf2d7` (5): the broker accepts only a descriptor it can PROVE to be shmem or a dma-buf and
/// closes any other on a helper thread (a FUSE file's `close` can hang) — so the relay sends
/// nothing else: a slot whose "dma-buf" is a pipe never reaches the broker; the check counts it and
/// says why. ⊘ CORRECTED 2026-10-04 (the review): the frame used to be refused on the rung `choose`
/// picked — every frame, for good, a black display with nothing tripping. Now the refused rung
/// backs off for the connection and the SAME frame goes on the next one: the dma-buf rung's to
/// shared memory, the GPU copy's to the host rungs. Nothing is spent before the check: the first
/// ATTACH that goes carries seq 0, and no line claims a rung that carried nothing.
/// Known-positive: the same slots with a memfd stand-in go LINEAR (and the GPU copy goes native).
#[test]
fn a_descriptor_that_is_neither_memfd_nor_dma_buf_never_reaches_the_broker() {
    for bad in [false, true] {
        // a pipe per slot: a distinct inode each (the ring refuses a shared one), and neither
        // shmem nor a dma-buf on any filesystem /tmp may be
        let mut t = T::with_dmabuf(|| {
            Some(if bad {
                OwnedFd::from(std::io::pipe().expect("pipe").0)
            } else {
                SharedRam::create_named(c"kfb-test-dmabuf", 64 * 1024)
                    .unwrap()
                    .dup_for_export()
                    .unwrap()
            })
        });
        t.up(CAP_MODIFIERS | CAP_RELEASE);
        let log = kf_broker::capture_log();
        let j = t.publish(64, 32);
        t.frame();
        let c = t.relay.counters();
        let att = t.attach_sent();
        assert_eq!(
            att.0.seq, 0,
            "bad={bad}: nothing was spent before the check"
        );
        if bad {
            assert_eq!(
                (att.0.flags, att.1),
                (CMD_F_SHM, Some(t.memfd_id(j))),
                "the same frame, as shared memory"
            );
            assert!(
                t.sent().iter().all(|(_, id)| *id != Some(t.dmabuf_id(j))),
                "the pipe never went"
            );
            assert_eq!((c.carrier_refused, c.refused), (1, 0));
            assert!(
                log.lines()
                    .iter()
                    .any(|l| l.contains("neither a memfd nor a dma-buf") && l.contains("5 s")),
                "{:?}",
                log.lines()
            );
            assert!(
                !log.lines().iter().any(|l| l.contains("LINEAR dma-buf")),
                "no rung announced that carried nothing: {:?}",
                log.lines()
            );
            assert!(t.relay.status().contains("carrier_refused=1"));
            assert!(t.relay.active(), "the connection stays up");
            // backing off: the next frame goes as shared memory without a second refusal
            t.release_shm(j);
            t.read();
            let (k, c2, id) = t.next_attach();
            assert_eq!((c2.flags, id), (CMD_F_SHM, Some(t.memfd_id(k))));
            assert_eq!(t.relay.counters().carrier_refused, 1);
        } else {
            assert_eq!(att.1, Some(t.dmabuf_id(j)));
            assert_eq!(c.carrier_refused, 0);
            assert!(t.relay.status().contains("carrier_refused=0"));
        }
    }
    // the GPU copy: a VRAM "dma-buf" that is a pipe backs the rung off; the frame takes a host rung
    for bad in [false, true] {
        let t0 = T::new(false);
        for j in 0..t0.ring.slots() {
            let fd = if bad {
                OwnedFd::from(std::io::pipe().expect("pipe").0)
            } else {
                SharedRam::create_named(c"kfb-test-vram", 64 * 1024)
                    .unwrap()
                    .dup_for_export()
                    .unwrap()
            };
            t0.ring
                .install_vram(j, VramFds::new(fd, 64 * 1024).unwrap())
                .unwrap();
        }
        t0.ring.set_vram_modifier(BL);
        t0.ring.set_gpu_nodes(NODES.to_vec());
        let mut t = t0;
        t.up(NATIVE_CAPS);
        t.bl_verdict(true);
        t.pkt(EV_FRAME, 0, 0, 0, 0);
        t.read();
        t.clear();
        let log = kf_broker::capture_log();
        // a frame only in VRAM (no console demand, no D2H): no other rung holds it
        let v = t.publish_kinds(64, 32, false, true);
        t.frame();
        if bad {
            assert!(!t.types().contains(&CMD_ATTACH), "{:?}", t.types());
            let c = t.relay.counters();
            assert_eq!((c.carrier_refused, c.refused), (1, 1));
            assert!(
                log.lines()
                    .iter()
                    .any(|l| l.contains("REFUSED frame slot") && l.contains("no other rung")),
                "{:?}",
                log.lines()
            );
            assert_eq!(t.ring.held_mask() & (1 << v), 0, "its slot came back");
            assert!(
                !t.ring.want_vram(),
                "the worker stops packing for the back-off"
            );
        } else {
            assert_eq!(t.attach_sent().1, Some(t.vram_id(v)), "native");
            t.release_vram(v);
            t.pkt(EV_FRAME, 0, 0, 0, 0);
            t.read();
        }
        t.clear();
        let j = t.publish_kinds(64, 32, true, true);
        t.frame();
        let att = t.attach_sent();
        if bad {
            assert_eq!(att.1, Some(t.memfd_id(j)), "the host rung carried it");
            assert_eq!(
                t.relay.counters().carrier_refused,
                1,
                "backed off: no re-check"
            );
            assert!(!t.ring.want_vram());
        } else {
            assert_eq!(att.1, Some(t.vram_id(j)), "native");
            assert!(t.ring.want_vram());
        }
    }
}

/// ★ The descriptor check reads `/proc/self/fdinfo` once per BACKING, not once per frame (the
/// review, 2026-10-04: a per-frame filesystem open on QEMU's main thread, under the relay's lock).
/// Known-positive: each backing is checked — the first frame of each slot runs the check.
#[test]
fn a_proven_descriptor_is_checked_once_per_backing() {
    let mut t = T::new(true);
    t.up(CAP_MODIFIERS | CAP_RELEASE);
    let mut used = std::collections::BTreeSet::new();
    for _ in 0..30 {
        let j = t.publish(64, 32);
        t.frame();
        assert_eq!(t.attach_sent().1, Some(t.dmabuf_id(j)), "LINEAR");
        used.insert(j);
        t.release_dmabuf(j);
        t.pkt(EV_FRAME, 0, 0, 0, 0);
        t.read();
        t.clear();
    }
    let checks = t.relay.counters().carrier_checks;
    assert!(
        checks >= 1 && checks <= used.len() as u64,
        "{checks} checks for {} backings over 30 frames",
        used.len()
    );
}

/// ★ The review of 2026-10-04: a dma-buf commit the broker REFUSED BY NAME (`EV_FORMAT x=0` for its
/// pair — the first frames of a connection race the relay's own question and are dropped at the
/// broker's format gate) is an answer, not a silent drop, and must not count toward the dma-buf
/// detector — whose trip blames `/proc/self/fdinfo`. Here two LINEAR XR24 frames are refused by
/// name, then AR24 frames go LINEAR, acknowledged: no trip. Known-positive: the same two frames
/// dropped WITHOUT a word trip the detector as soon as a third dma-buf commit goes.
#[test]
fn a_dma_buf_frame_refused_by_name_never_trips_the_detector() {
    for told in [false, true] {
        let mut t = T::new(true);
        t.up(CAP_MODIFIERS | CAP_RELEASE);
        let log = kf_broker::capture_log();
        let t0 = t.now;
        let a = t.publish(64, 32);
        t.frame();
        assert_eq!(t.attach_sent().1, Some(t.dmabuf_id(a)), "LINEAR, asked");
        t.pkt(EV_FRAME, 0, 0, 0, 0);
        t.read();
        t.clear();
        let b = t.publish(64, 32);
        t.frame();
        assert_eq!(t.attach_sent().1, Some(t.dmabuf_id(b)), "LINEAR again");
        if told {
            t.pkt(EV_FORMAT, 0, FOURCC_XR24 as i32, 0, 0);
            t.read();
            assert_eq!(t.ring.held_mask(), 0, "both refused frames came back");
        }
        // a second pair the relay sends, after the detector's window
        t.now = t0 + 1100;
        t.clear();
        let c = t.publish_fourcc(64, 32, FOURCC_AR24);
        t.frame();
        for _ in 0..5 {
            if t.types().contains(&CMD_ATTACH) {
                break;
            }
            t.tick(100);
        }
        let att = t.attach_sent();
        assert_eq!(
            (att.0.fourcc, att.1),
            (FOURCC_AR24, Some(t.dmabuf_id(c))),
            "told={told}"
        );
        t.tick(10);
        let trips = t.relay.counters().dmabuf_trips;
        if told {
            t.release_dmabuf(c);
            t.read();
            t.tick(3000);
            assert_eq!(t.relay.counters().dmabuf_trips, 0, "{:?}", log.lines());
            assert!(
                !log.lines().iter().any(|l| l.contains("/proc/self/fdinfo")),
                "{:?}",
                log.lines()
            );
            assert!(t.relay.status().contains("dmabuf_trips=0"));
        } else {
            assert_eq!(trips, 1, "three silent drops trip it");
            assert!(t.relay.status().contains("dmabuf_trips=1"));
        }
    }
}

/// ★ The review of 2026-10-04: sixteen volunteered "no"s used to evict the relay's own recorded
/// YES for the GPU-copy pair (the table dropped its oldest non-"no" first) — and with it the
/// rung, the worker's pack, and a fresh question per frame. The broker's volunteered rows go
/// first now. Known-positive: the GPU copy is in use before the "no"s arrive, and still after.
#[test]
fn volunteered_noes_never_push_out_the_relays_own_yes() {
    let mut t = T::with_vram();
    t.up(NATIVE_CAPS);
    t.bl_verdict(true);
    t.pkt(EV_FRAME, 0, 0, 0, 0);
    t.read();
    assert!(t.ring.want_vram());
    t.clear();
    let j = t.publish_kinds(64, 32, true, true);
    t.frame();
    assert_eq!(t.attach_sent().1, Some(t.vram_id(j)), "native before");
    t.release_vram(j);
    for m in 0..32u32 {
        t.pkt(EV_FORMAT, 0, FOURCC_AR24 as i32, 0x100 + m, 0x0300_0000);
    }
    t.pkt(EV_FRAME, 0, 0, 0, 0);
    t.read();
    assert_eq!(t.relay.counters().formats_unasked, 32);
    assert!(t.relay.status().contains("formats_unasked=32"));
    assert!(t.ring.want_vram(), "the yes is still known");
    t.clear();
    let k = t.publish_kinds(64, 32, true, true);
    t.frame();
    assert!(
        !t.types().contains(&CMD_QUERY_FORMAT),
        "not asked again: {:?}",
        t.types()
    );
    assert_eq!(t.attach_sent().1, Some(t.vram_id(k)), "native after");
}

/// ★ The HELLO line says "shared memory" only when installed slots ALL lack a dma-buf (⊘ box run
/// `brkF1`, 2026-10-04: it said "no /dev/udmabuf here" at the first connection, before the guest's
/// first frame, when no slot holds anything — and the next frames went LINEAR). Known-positive: a
/// ring whose slots have no dma-buf says it.
#[test]
fn the_hello_line_claims_shared_memory_only_for_slots_without_a_dma_buf() {
    for (what, says) in [("empty", false), ("memfd only", true), ("dma-buf", false)] {
        let mut t = match what {
            "empty" => T::over(Arc::new(FrameRing::new(
                kf_broker::slots::BROKER_SLOTS,
                true,
            ))),
            "memfd only" => T::new(false),
            _ => T::new(true),
        };
        let log = kf_broker::capture_log();
        t.up(CAP_MODIFIERS);
        let hello: Vec<_> = log
            .lines()
            .into_iter()
            .filter(|l| l.starts_with("connected to"))
            .collect();
        assert_eq!(hello.len(), 1, "{what}");
        assert_eq!(
            hello[0].contains("shared memory"),
            says,
            "{what}: {hello:?}"
        );
    }
}

// ── §8.18: the buffer path each display environment ends on (owner, 2026-10-08) ─────────────

/// Which path the last ATTACH of slot `j` took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BufPath {
    Native,
    Linear,
    Implicit,
    Shm,
}

impl T {
    /// One frame, after the broker returned the pacing credit; the path its ATTACH took.
    fn frame_path(&mut self, host: bool, vram: bool) -> BufPath {
        // a displaying broker gives every shown frame back: RELEASE each held slot under each
        // of its backings (an id the relay did not send on that slot is ignored)
        let held = self.ring.held_mask();
        for j in (0..self.ring.slots()).filter(|j| held & (1 << j) != 0) {
            let mut ids = vec![self.memfd_id(j)];
            ids.extend(self.ring.fds(j).unwrap().dmabuf_id());
            ids.extend(self.ring.vram(j).map(|v| v.id()));
            for id in ids {
                self.pkt(EV_RELEASE, 0, 0, id as u32, (id >> 32) as u32);
            }
        }
        self.pkt(EV_FRAME, 0, 0, 0, 0);
        self.read();
        self.clear();
        let j = self.publish_kinds(64, 32, host, vram);
        self.frame();
        let (c, id) = self
            .sent()
            .into_iter()
            .find(|(c, _)| c.ty == CMD_ATTACH)
            .expect("a frame always goes: a refusal must never stall the display");
        assert!(
            self.types().contains(&CMD_COMMIT),
            "an ATTACH is committed: {:?}",
            self.types()
        );
        if c.modifier == BL && id == self.ring.vram(j).map(|v| v.id()) {
            BufPath::Native
        } else if c.flags == CMD_F_SHM && id == Some(self.memfd_id(j)) {
            BufPath::Shm
        } else if c.flags == 0 && id == self.ring.fds(j).unwrap().dmabuf_id() {
            match c.modifier {
                MOD_LINEAR => BufPath::Linear,
                MOD_INVALID => BufPath::Implicit,
                m => panic!("a dma-buf under modifier {m:#x}"),
            }
        } else {
            panic!("an ATTACH of no known backing: {c:?} {id:?}")
        }
    }

    /// The display's answer for (XR24, `modifier`).
    fn verdict(&self, modifier: u64, yes: bool) {
        self.pkt(
            EV_FORMAT,
            i32::from(yes),
            FOURCC_XR24 as i32,
            modifier as u32,
            (modifier >> 32) as u32,
        );
    }
}

/// ★ Environment 1, measured on the trusted host (run wl1, 2026-10-08): GNOME Wayland on the
/// NVIDIA GPU that renders the guest. Native block-linear YES, LINEAR advertised and then
/// refused at import (mutter's import fails; `badf2d7` sends the unsolicited `x=0`), shm YES.
/// The path must end native, pass through shm (never LINEAR again) while the native answer is
/// pending, and land on shm — not a stall — when the native path is later refused too.
#[test]
fn env_native_display_ends_native_and_falls_back_to_shm_never_linear_again() {
    let mut t = T::vram_over(T::new(true));
    t.up(NATIVE_CAPS);
    assert_eq!(
        t.frame_path(true, false),
        BufPath::Linear,
        "unanswered: optimistic"
    );
    t.verdict(MOD_LINEAR, true); // advertised …
    t.read();
    t.verdict(MOD_LINEAR, false); // … and refused at import
    t.read();
    assert_eq!(
        t.frame_path(true, false),
        BufPath::Shm,
        "the refusal lands on shm"
    );
    t.bl_verdict(true);
    t.read();
    assert_eq!(t.frame_path(true, true), BufPath::Native);
    let j = t.ring.held_mask().trailing_zeros() as usize;
    t.release_vram(j);
    assert_eq!(
        t.frame_path(true, true),
        BufPath::Native,
        "and stays native"
    );
    t.bl_verdict(false); // the native import refused later
    t.read();
    assert_eq!(
        t.frame_path(true, true),
        BufPath::Shm,
        "never LINEAR: it was refused"
    );
    assert_eq!(t.frame_path(true, true), BufPath::Shm);
}

/// ★ Environment 2, MODELLED (no such hardware was run): a laptop whose compositor runs on an
/// Intel iGPU, the NVIDIA dGPU with no display. The compositor names its own render node
/// (EV_DEVICE, not this GPU), advertises no NVIDIA modifier (the block-linear pair: no) and
/// imports LINEAR. The path must end LINEAR; a later LINEAR refusal lands on shm.
#[test]
fn env_other_gpu_compositor_ends_linear_and_falls_back_to_shm() {
    use kf_broker::wire::{CAP_DEVICE, DEVICE_F_KNOWN, DEVICE_F_RENDER, EV_DEVICE};
    let mut t = T::vram_over(T::new(true));
    t.up(NATIVE_CAPS | CAP_DEVICE);
    t.pkt(EV_DEVICE, DEVICE_F_KNOWN | DEVICE_F_RENDER, 0, 226, 130);
    t.bl_verdict(false);
    t.read();
    assert!(
        !t.ring.want_vram(),
        "no GPU copy for another GPU's compositor"
    );
    t.verdict(MOD_LINEAR, true);
    t.read();
    assert_eq!(t.frame_path(true, true), BufPath::Linear);
    assert_eq!(t.frame_path(true, true), BufPath::Linear);
    t.verdict(MOD_LINEAR, false);
    t.read();
    assert_eq!(t.frame_path(true, true), BufPath::Shm);
}

/// ★ Environment 3, MODELLED: Xvfb / a headless llvmpipe compositor. No explicit modifiers (no
/// `CAP_MODIFIERS`, so neither the native pair nor LINEAR is asked), and the implicit-modifier
/// dma-buf refused: shm from the first frame on. With no `/dev/udmabuf` at all the same.
#[test]
fn env_software_display_ends_on_shm() {
    let mut t = T::vram_over(T::new(true));
    t.up(CAP_RELEASE);
    assert_eq!(t.frame_path(true, false), BufPath::Shm, "unanswered: shm");
    t.verdict(MOD_INVALID, false);
    t.read();
    assert_eq!(t.frame_path(true, true), BufPath::Shm);
    assert!(!t.ring.want_vram());
    // a "no" is final for the connection (nvkvm-pv's `relay_format_verdict_next` keeps a 0): a
    // later yes does not move it off shm
    t.verdict(MOD_INVALID, true);
    t.read();
    assert_eq!(t.frame_path(true, true), BufPath::Shm);
    // a display that says yes to the implicit modifier gets the dma-buf (rung 1b)
    let mut t = T::vram_over(T::new(true));
    t.up(CAP_RELEASE);
    assert_eq!(t.frame_path(true, false), BufPath::Shm, "asked; unanswered");
    t.verdict(MOD_INVALID, true);
    t.read();
    assert_eq!(t.frame_path(true, false), BufPath::Implicit);
    let mut t = T::vram_over(T::new(false));
    t.up(CAP_MODIFIERS | CAP_RELEASE);
    for _ in 0..3 {
        assert_eq!(t.frame_path(true, false), BufPath::Shm, "no udmabuf: shm");
    }
}

// ── §8.19: the grab routing invariant (owner, 2026-10-08: Minecraft's mouse-look) ───────────

/// A packet carrying the broker's grab state, as every packet does (`F_GRABBED`).
fn grabbed_pkt(t: &T, ty: u16, x: i32, y: i32, w0: u32, w1: u32) {
    let p = Pkt {
        ty,
        flags: kf_broker::wire::F_GRABBED,
        x,
        y,
        w0,
        w1,
        ..Pkt::default()
    };
    t.wire.borrow_mut().inbox.extend(p.encode());
}

/// ★ While grabbed: motion, buttons and the wheel all go to the RELATIVE pointer, no absolute
/// report is generated (one the broker sends anyway is dropped), and the grab's end hands the
/// absolute device the last position before the grab (where the host's locked pointer is).
/// Outside a grab the same buttons and wheel go to the absolute pointer, as before.
#[test]
fn under_grab_every_pointer_event_goes_to_the_relative_device_and_no_abs() {
    let mut t = T::new(false);
    t.up(0);
    t.pkt(EV_ABS, 300, 200, 1920, 1080);
    t.pkt(EV_BTN, 272, 1, 0, 0);
    t.pkt(EV_WHEEL, 1, 0, 0, 0);
    let before = t.read();
    assert_eq!(
        before,
        vec![
            Input::Abs {
                x: 300,
                y: 200,
                w: 1920,
                h: 1080
            },
            Input::Btn {
                code: 272,
                down: true,
                to: Pointer::Absolute
            },
            Input::Wheel {
                dx: 0,
                dy: 1,
                to: Pointer::Absolute
            },
        ]
    );
    // the grab (EV_GRAB carries F_GRABBED too) and what follows it
    grabbed_pkt(&t, EV_GRAB, 1, 0, 0, 0);
    grabbed_pkt(&t, EV_REL, 5, 0, 0, 0);
    grabbed_pkt(&t, EV_REL, 5, -2, 0, 0);
    grabbed_pkt(&t, EV_BTN, 272, 0, 0, 0);
    grabbed_pkt(&t, EV_ABS, 1700, 900, 1920, 1080); // a broker that sends one anyway
    grabbed_pkt(&t, EV_WHEEL, -1, 0, 0, 0);
    grabbed_pkt(&t, EV_BTN, 273, 1, 0, 0);
    let during = t.read();
    assert_eq!(
        during,
        vec![
            Input::Grab(true),
            Input::Rel { dx: 10, dy: -2 },
            Input::Btn {
                code: 272,
                down: false,
                to: Pointer::Relative
            },
            Input::Wheel {
                dx: 0,
                dy: -1,
                to: Pointer::Relative
            },
            Input::Btn {
                code: 273,
                down: true,
                to: Pointer::Relative
            },
        ],
        "no Abs while grabbed, every button and wheel tick to the relative device"
    );
    // the end of the grab: the absolute device is re-synced to the pre-grab position
    t.pkt(EV_GRAB, 0, 0, 0, 0);
    t.pkt(EV_BTN, 273, 0, 0, 0);
    assert_eq!(
        t.read(),
        vec![
            Input::Grab(false),
            Input::Abs {
                x: 300,
                y: 200,
                w: 1920,
                h: 1080
            },
            Input::Btn {
                code: 273,
                down: false,
                to: Pointer::Absolute
            },
        ]
    );
}

/// ★ The once-a-second count of what arrived while grabbed — the instrument for "does an absolute
/// report or a button on the absolute device reach the guest during mouse-look?".
#[test]
fn input_while_grabbed_is_counted_and_logged_once_a_second() {
    let mut t = T::new(false);
    t.up(0);
    let log = kf_broker::capture_log();
    grabbed_pkt(&t, EV_GRAB, 1, 0, 0, 0);
    for _ in 0..3 {
        grabbed_pkt(&t, EV_REL, 1, 1, 0, 0);
    }
    grabbed_pkt(&t, EV_ABS, 1, 1, 10, 10);
    grabbed_pkt(&t, EV_BTN, 272, 1, 0, 0);
    t.read();
    assert!(
        !log.lines()
            .iter()
            .any(|l| l.contains("input while grabbed")),
        "not before a second"
    );
    t.now += 1_000;
    grabbed_pkt(&t, EV_REL, 1, 0, 0, 0);
    t.read();
    let l = log.lines();
    let line = l
        .iter()
        .find(|l| l.contains("input while grabbed"))
        .expect("logged after a second");
    assert!(
        line.contains("1 button")
            && line.contains("4 REL packet")
            && line.contains("1 absolute report(s) dropped"),
        "{line}"
    );
    // the end of the grab flushes the interval's count
    grabbed_pkt(&t, EV_KEY, 30, 1, 0, 0);
    t.pkt(EV_GRAB, 0, 0, 0, 0);
    t.read();
    assert!(
        log.lines()
            .iter()
            .any(|l| l.contains("1 key") && l.contains("grab ended")),
        "{:?}",
        log.lines()
    );
}
