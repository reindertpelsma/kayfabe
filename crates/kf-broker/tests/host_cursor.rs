// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ The guest's cursor as the host pointer (`OWNER_RULINGS.md` §O, `docs/design/V3_DISPLAY.md`
//! §8.12) — the relay against a scripted link: what reaches the broker, and the mode the worker
//! composes by. Each test has a known-positive half (the same script with the one thing that
//! decides it changed), so a detector that never fires cannot pass.

use kf_broker::console::DEFINE_MIN_MS;
use kf_broker::wire::{
    CAP_CURSOR, CAP_DMABUF, CMD_CURSOR, CMD_SIZE, CURSOR_HIDE, CURSOR_SET, CURSOR_SHOW, EV_ABS,
    EV_FOCUS, EV_GRAB, EV_HELLO, EV_KEY, F_GRABBED, FOURCC_AR24, Pkt,
};
use kf_broker::{
    ConsoleCursor, CursorImage, CursorMode, CursorShare, CursorWant, FrameRing, Host, Link,
    PointerAbs, Recv, Relay, RelayConfig, Sent, ShownFrame, SlotFds,
};
use kf_linux_raw::{SharedRam, fd_inode};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::os::fd::{BorrowedFd, OwnedFd};
use std::os::unix::fs::{FileExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

/// One record the relay sent: the raw 40 bytes, and for a descriptor its inode, its size and its
/// bytes — read back from a duplicate, the way the broker `pread`s it.
#[derive(Debug, Clone)]
struct Rec {
    raw: [u8; CMD_SIZE],
    fd: Option<(u64, u64, Vec<u8>)>,
}

impl Rec {
    fn ty(&self) -> u16 {
        u16::from_le_bytes([self.raw[0], self.raw[1]])
    }
    fn u32_at(&self, at: usize) -> u32 {
        u32::from_le_bytes(self.raw[at..at + 4].try_into().unwrap())
    }
    /// `(width, height, stride, offset, fourcc, hot_x, hot_y, op)` of a cursor record.
    fn cursor(&self) -> [u32; 8] {
        [4, 8, 12, 16, 20, 24, 28, 32].map(|at| self.u32_at(at))
    }
}

#[derive(Default)]
struct Wire {
    inbox: VecDeque<u8>,
    eof: bool,
    sent: Vec<Rec>,
    budget: Option<usize>,
    /// The duplicates of every descriptor sent — the broker's copies, kept open.
    held: Vec<OwnedFd>,
}

#[derive(Clone)]
struct Fake(Rc<RefCell<Wire>>);
struct Sock;

impl Link for Fake {
    type Sock = Sock;
    fn connect(&mut self, _path: &Path) -> Result<Sock, String> {
        Ok(Sock)
    }
    fn fd(&self, _s: &Sock) -> i32 {
        77
    }
    fn peer_uid(&mut self, _s: &Sock) -> Result<u32, String> {
        Ok(1000)
    }
    fn effective_uid(&mut self) -> u32 {
        1000
    }
    fn send(&mut self, _s: &Sock, rec: &[u8; CMD_SIZE], fd: Option<BorrowedFd<'_>>) -> Sent {
        let mut w = self.0.borrow_mut();
        if let Some(b) = w.budget.as_mut() {
            if *b == 0 {
                return Sent::Full;
            }
            *b -= 1;
        }
        let fd = fd.map(|f| {
            let id = fd_inode(f).expect("a live descriptor");
            let dup = f.try_clone_to_owned().expect("dup");
            let file = std::fs::File::from(dup.try_clone().expect("dup"));
            let len = file.metadata().expect("fstat").len();
            let mut b = vec![0u8; usize::try_from(len).unwrap()];
            file.read_exact_at(&mut b, 0).expect("pread");
            w.held.push(dup);
            (id, len, b)
        });
        w.sent.push(Rec { raw: *rec, fd });
        Sent::Done
    }
    fn recv(&mut self, _s: &Sock, buf: &mut [u8]) -> Recv {
        let mut w = self.0.borrow_mut();
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
}

impl Host for H {
    fn watch(&mut self, fd: i32, read: bool, write: bool) {
        self.watches.push((fd, read, write));
    }
    fn timer(&mut self, _d: Option<u64>) {}
}

struct T {
    wire: Rc<RefCell<Wire>>,
    relay: Relay<Fake>,
    share: Arc<CursorShare>,
    host: H,
    now: u64,
}

impl T {
    /// A relay with the cursor share attached (`share` = false: without it — today's relay).
    fn new(share: bool) -> T {
        let ring = Arc::new(FrameRing::new(kf_broker::slots::BROKER_SLOTS, true));
        for j in 0..ring.slots() {
            let mem = SharedRam::create_named(c"kfb-cursor-test-frame", 64 * 1024).unwrap();
            ring.install(j, SlotFds::new(mem, None).unwrap()).unwrap();
        }
        let wire = Rc::new(RefCell::new(Wire::default()));
        let cursor = Arc::new(CursorShare::new());
        let relay = Relay::new(
            RelayConfig {
                path: PathBuf::from("/run/test/display.sock"),
                extra_uid: None,
            },
            ring,
            Fake(wire.clone()),
        );
        let relay = if share {
            relay.with_cursor(cursor.clone())
        } else {
            relay
        };
        T {
            wire,
            relay,
            share: cursor,
            host: H::default(),
            now: 1_000,
        }
    }

    fn pkt(&self, ty: u16, flags: u16, x: i32) {
        let p = Pkt {
            ty,
            flags,
            x,
            ..Pkt::default()
        };
        self.wire.borrow_mut().inbox.extend(p.encode());
    }

    fn read(&mut self) {
        let mut out = Vec::new();
        let now = self.now;
        self.relay
            .on_socket(now, true, false, &mut self.host, &mut out, 64);
    }

    /// Connect and HELLO with `caps` (plus `CAP_DMABUF`), `flags` on the HELLO; forget the replay.
    fn up(&mut self, caps: u32, flags: u16) {
        let now = self.now;
        self.relay.start(now, &mut self.host);
        self.relay.on_timer(now, &mut self.host);
        let p = Pkt {
            ty: EV_HELLO,
            flags,
            w0: 2,
            w1: caps | CAP_DMABUF,
            ..Pkt::default()
        };
        self.wire.borrow_mut().inbox.extend(p.encode());
        self.read();
        assert!(self.relay.active());
        self.wire.borrow_mut().sent.clear();
    }

    /// One relay entry (the frame publish that follows every post wakes it).
    fn tick(&mut self) {
        self.now += 16;
        let now = self.now;
        self.relay.on_frame(now, &mut self.host);
    }

    fn cursors(&self) -> Vec<Rec> {
        self.wire
            .borrow()
            .sent
            .iter()
            .filter(|r| r.ty() == CMD_CURSOR)
            .cloned()
            .collect()
    }

    fn ops(&self) -> Vec<u32> {
        self.cursors().iter().map(|r| r.cursor()[7]).collect()
    }

    fn clear(&self) {
        self.wire.borrow_mut().sent.clear();
    }
}

/// A `w` x `h` premultiplied image whose pixel `i` is `i + seed` in every byte (alpha included).
fn image(w: u32, h: u32, hot: (u32, u32), seed: u8) -> CursorWant {
    let px = (0..w * h * 4)
        .map(|i| (i as u8).wrapping_add(seed))
        .collect();
    CursorWant::Image(Arc::new(CursorImage::new(w, h, hot, px).unwrap()))
}

fn pixels(w: &CursorWant) -> Vec<u8> {
    match w {
        CursorWant::Image(i) => i.pixels().to_vec(),
        _ => panic!("not an image"),
    }
}

/// The descriptors in this process naming inode `id` (each `/proc/self/fd` entry stat'ed through).
fn open_copies(id: u64) -> usize {
    std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| std::fs::metadata(e.path()).is_ok_and(|m| m.ino() == id))
        .count()
}

// ── the capability ──────────────────────────────────────────────────────────────────────────

/// ★ A broker without `CAP_CURSOR` is never sent a `CMD_CURSOR` (an older broker treats the type
/// as a violation) and the worker keeps composing; WITH the bit the same script sends the SET —
/// the known-positive. And a relay without the share is today's relay whatever the broker says.
#[test]
fn a_broker_without_cap_cursor_is_never_sent_one_and_the_cursor_stays_composed() {
    for (caps, share, want_set) in [
        (0, true, false),
        (CAP_CURSOR, true, true),
        (CAP_CURSOR, false, false),
    ] {
        let mut t = T::new(share);
        t.up(caps, 0);
        assert!(t.share.post(image(32, 32, (1, 1), 3)));
        t.tick();
        t.tick();
        let expect = if want_set { vec![CURSOR_SET] } else { vec![] };
        assert_eq!(t.ops(), expect, "caps {caps:#x} share {share}");
        let mode = if want_set {
            CursorMode::Hover
        } else {
            CursorMode::Off
        };
        assert_eq!(
            t.share.mode(),
            mode,
            "caps {caps:#x} share {share}: what the worker composes by"
        );
    }
}

// ── SET, and the change detection ───────────────────────────────────────────────────────────

/// ★ Hover: the image goes ONCE, as the broker's record says (premultiplied AR24, tight stride,
/// offset 0, the hot spot) in a memfd holding exactly its pixels, which kf closed after sending;
/// an unchanged image (even re-posted) is never re-sent, and a new hot spot alone is a new SET.
#[test]
fn hover_sends_the_image_once_in_a_memfd_and_again_only_when_it_or_its_hot_spot_changes() {
    let mut t = T::new(true);
    t.up(CAP_CURSOR, 0);
    assert_eq!(t.share.mode(), CursorMode::Hover);
    let a = image(64, 48, (3, 5), 9);
    assert!(t.share.post(a.clone()));
    t.tick();
    let c = t.cursors();
    assert_eq!(c.len(), 1, "one SET");
    assert_eq!(
        c[0].cursor(),
        [64, 48, 256, 0, FOURCC_AR24, 3, 5, CURSOR_SET]
    );
    assert_eq!(&c[0].raw[2..4], &[0, 0], "flags");
    assert_eq!(&c[0].raw[36..40], &[0, 0, 0, 0], "reserved1");
    let (id, len, bytes) = c[0].fd.clone().expect("a SET carries a memfd");
    assert_eq!(len, 64 * 48 * 4, "the memfd is the image");
    assert_eq!(bytes, pixels(&a), "the memfd holds exactly the pixels");
    assert_eq!(
        open_copies(id),
        1,
        "only the broker's copy is open: kf closed its own after sending"
    );
    // unchanged — re-posted, and posted as an equal image: nothing
    t.clear();
    assert!(t.share.post(a.clone()));
    t.tick();
    assert!(t.share.post(image(64, 48, (3, 5), 9)));
    t.tick();
    t.tick();
    assert_eq!(
        t.ops(),
        Vec::<u32>::new(),
        "an unchanged cursor is not re-sent"
    );
    // the hot spot alone
    assert!(t.share.post(image(64, 48, (4, 5), 9)));
    t.tick();
    assert_eq!(t.ops(), vec![CURSOR_SET], "a new hot spot is a new SET");
    // the pixels alone
    t.clear();
    assert!(t.share.post(image(64, 48, (4, 5), 10)));
    t.tick();
    assert_eq!(t.ops(), vec![CURSOR_SET], "a new image is a new SET");
    assert_eq!(t.relay.counters().cursor_sets, 3);
}

/// ★ No cursor is a HIDE; the image the broker holds coming back is a SHOW, never a second SET; a
/// cursor kf composes itself (XOR) hides the host's too; HIDE and SHOW carry no descriptor.
#[test]
fn hidden_is_a_hide_and_the_held_image_coming_back_is_a_show() {
    let mut t = T::new(true);
    t.up(CAP_CURSOR, 0);
    let a = image(32, 32, (0, 0), 1);
    for (want, op) in [
        (CursorWant::Hidden, None),
        (a.clone(), Some(CURSOR_SET)),
        (CursorWant::Hidden, Some(CURSOR_HIDE)),
        (CursorWant::Hidden, None),
        (a.clone(), Some(CURSOR_SHOW)),
        (CursorWant::Composed, Some(CURSOR_HIDE)),
        (a.clone(), Some(CURSOR_SHOW)),
    ] {
        t.clear();
        assert!(t.share.post(want.clone()));
        t.tick();
        assert_eq!(t.ops(), op.into_iter().collect::<Vec<_>>(), "{want:?}");
        for r in t.cursors() {
            if r.cursor()[7] != CURSOR_SET {
                assert!(r.fd.is_none(), "HIDE/SHOW carry no fd");
                assert!(
                    r.raw[2..32].iter().all(|b| *b == 0),
                    "HIDE/SHOW: every field but op zero"
                );
            }
        }
    }
    let k = t.relay.counters();
    assert_eq!((k.cursor_sets, k.cursor_hides, k.cursor_shows), (1, 2, 2));
}

// ── grab ────────────────────────────────────────────────────────────────────────────────────

/// ★ The hover/grab switch: under grab the worker composes (mode `Grabbed`) and the broker is
/// sent NOTHING — it hides its image itself — however the cursor changes; the grab's end brings
/// back hover and the cursor that changed meanwhile goes at once. The grab is read from `EV_GRAB`
/// AND from `F_GRABBED` on any other packet (`proto.h`: mirrored on every packet).
#[test]
fn grab_composes_and_sends_nothing_and_its_end_sends_what_changed() {
    let mut t = T::new(true);
    t.up(CAP_CURSOR, 0);
    let a = image(32, 32, (0, 0), 1);
    let b = image(32, 32, (0, 0), 2);
    assert!(t.share.post(a.clone()));
    t.tick();
    assert_eq!(t.ops(), vec![CURSOR_SET]);
    // grab on (EV_GRAB): composed, silent
    t.clear();
    t.pkt(EV_GRAB, F_GRABBED, 1);
    t.read();
    assert_eq!(t.share.mode(), CursorMode::Grabbed);
    assert!(t.share.post(b.clone()));
    t.tick();
    assert!(t.share.post(CursorWant::Hidden));
    t.tick();
    assert!(t.share.post(b.clone()));
    t.tick();
    assert_eq!(t.ops(), Vec::<u32>::new(), "nothing is sent under grab");
    // the grab ends, seen only in F_GRABBED on an ordinary packet: hover, and b goes at once
    t.pkt(EV_FOCUS, 0, 1);
    t.read();
    assert_eq!(t.share.mode(), CursorMode::Hover);
    assert_eq!(
        t.ops(),
        vec![CURSOR_SET],
        "the cursor that changed under grab"
    );
    assert_eq!(t.cursors()[0].fd.as_ref().unwrap().2, pixels(&b));
    // F_GRABBED on an ordinary packet starts a grab too
    t.clear();
    t.pkt(EV_KEY, F_GRABBED, 30);
    t.read();
    assert_eq!(t.share.mode(), CursorMode::Grabbed, "F_GRABBED alone");
    assert!(t.share.post(a.clone()));
    t.tick();
    assert_eq!(t.ops(), Vec::<u32>::new());
    // EV_GRAB x=0 ends it, whatever that packet's flags say
    t.pkt(EV_GRAB, F_GRABBED, 0);
    t.read();
    assert_eq!(t.share.mode(), CursorMode::Hover);
    assert_eq!(t.ops(), vec![CURSOR_SET]);
}

/// A broker that comes up grabbed (`F_GRABBED` on its HELLO) starts in `Grabbed`.
#[test]
fn a_broker_that_says_hello_grabbed_starts_composed() {
    let mut t = T::new(true);
    t.up(CAP_CURSOR, F_GRABBED);
    assert_eq!(t.share.mode(), CursorMode::Grabbed);
    assert!(t.share.post(image(32, 32, (0, 0), 1)));
    t.tick();
    assert_eq!(t.ops(), Vec::<u32>::new());
}

// ── the rate bound ──────────────────────────────────────────────────────────────────────────

/// ★ Coalescing: however many cursors the worker posts between two relay entries, the broker
/// gets ONE SET — of the newest — and each entry sends at most one command.
#[test]
fn posts_between_two_entries_coalesce_to_one_set_of_the_newest() {
    let mut t = T::new(true);
    t.up(CAP_CURSOR, 0);
    let mut last = CursorWant::Hidden;
    for seed in 0..10u8 {
        last = image(32, 32, (0, 0), seed);
        assert!(t.share.post(last.clone()));
    }
    t.tick();
    assert_eq!(t.ops(), vec![CURSOR_SET], "one SET for ten posts");
    assert_eq!(t.cursors()[0].fd.as_ref().unwrap().2, pixels(&last));
    t.tick();
    t.tick();
    assert_eq!(t.ops(), vec![CURSOR_SET], "and nothing after it");
    // an image and then no cursor, both before the entry: only the newest counts — one HIDE
    t.clear();
    assert!(t.share.post(image(32, 32, (0, 0), 77)));
    assert!(t.share.post(CursorWant::Hidden));
    t.tick();
    assert_eq!(t.ops(), vec![CURSOR_HIDE]);
}

// ── the socket ──────────────────────────────────────────────────────────────────────────────

/// ★ `EAGAIN` owes the cursor command: the watch asks for writability, and the command goes
/// exactly once when the socket drains.
#[test]
fn a_full_socket_owes_the_cursor_command_and_sends_it_once_writable() {
    let mut t = T::new(true);
    t.up(CAP_CURSOR, 0);
    t.wire.borrow_mut().budget = Some(0);
    assert!(t.share.post(image(32, 32, (0, 0), 1)));
    t.tick();
    assert_eq!(t.ops(), Vec::<u32>::new());
    assert_eq!(
        t.host.watches.last(),
        Some(&(77, true, true)),
        "writability is watched for the owed command"
    );
    t.wire.borrow_mut().budget = None;
    let mut out = Vec::new();
    let now = t.now;
    t.relay
        .on_socket(now, false, true, &mut t.host, &mut out, 64);
    assert_eq!(t.ops(), vec![CURSOR_SET]);
    assert_eq!(t.host.watches.last(), Some(&(77, true, false)));
    t.tick();
    assert_eq!(t.ops(), vec![CURSOR_SET], "once");
}

/// ★ The cursor is connection state on the broker's side: a new connection is sent the cursor
/// again, and while there is none the worker composes (mode `Off`).
#[test]
fn a_reconnect_is_sent_the_cursor_again_and_no_broker_composes() {
    let mut t = T::new(true);
    t.up(CAP_CURSOR, 0);
    let a = image(32, 32, (2, 2), 4);
    assert!(t.share.post(a.clone()));
    t.tick();
    assert_eq!(t.ops(), vec![CURSOR_SET]);
    t.wire.borrow_mut().eof = true;
    t.read();
    assert!(!t.relay.connected());
    assert_eq!(t.share.mode(), CursorMode::Off, "no broker: composed");
    t.wire.borrow_mut().eof = false;
    t.now += 10_000;
    let now = t.now;
    t.relay.on_timer(now, &mut t.host);
    t.clear();
    let p = Pkt {
        ty: EV_HELLO,
        w0: 2,
        w1: CAP_CURSOR | CAP_DMABUF,
        ..Pkt::default()
    };
    t.wire.borrow_mut().inbox.extend(p.encode());
    t.read();
    assert!(t.relay.active());
    assert_eq!(t.share.mode(), CursorMode::Hover);
    assert_eq!(
        t.ops(),
        vec![CURSOR_SET],
        "the new broker holds no cursor: sent again"
    );
    assert_eq!(t.cursors()[0].fd.as_ref().unwrap().2, pixels(&a));
}

/// ★ The relay hands the worker the absolute position it injected — clamped exactly like the input
/// it describes — from which the hot spot NVKMS does not program is derived (`HotTracker`).
#[test]
fn an_injected_absolute_position_reaches_the_cursor_share() {
    let mut t = T::new(true);
    t.up(CAP_CURSOR, 0);
    assert_eq!(t.share.abs(), (0, None));
    for (x, y, want) in [(703, 405, (703, 405)), (5000, -7, (1023, 0))] {
        let p = Pkt {
            ty: EV_ABS,
            x,
            y,
            w0: 1024,
            w1: 768,
            ..Pkt::default()
        };
        t.wire.borrow_mut().inbox.extend(p.encode());
        t.read();
        let (_, a) = t.share.abs();
        assert_eq!(
            a,
            Some(PointerAbs {
                x: want.0,
                y: want.1,
                w: 1024,
                h: 768
            })
        );
    }
    assert_eq!(t.share.abs().0, 2, "one per injected position");
}

/// ★ nvkvm-pv `badf2d7` (6): the broker paces cursor uploads ITSELF — at most one per 8 ms, latest
/// wins and the latest is always applied, and its backends see only the snapshot it publishes —
/// and scales the hot spot (`ceil(hot * out / in)`). So the relay adds no pacing of its own: each
/// entry that finds a newer cursor sends it, 1 ms after the last or not (a relay-side throttle
/// would only delay the hot-spot correction `HotTracker` makes), and the hot spot goes in GUEST
/// pixels. Known-positive: the same five posts made between two entries are ONE SET.
#[test]
fn the_relay_leaves_the_cursor_pacing_to_the_broker() {
    let mut t = T::new(true);
    t.up(CAP_CURSOR, 0);
    let mut want = Vec::new();
    for seed in 0..5u8 {
        let w = image(32, 32, (u32::from(seed), 1), seed);
        assert!(t.share.post(w.clone()));
        t.now += 1;
        let now = t.now;
        t.relay.on_frame(now, &mut t.host);
        want.push(w);
    }
    assert_eq!(
        t.ops(),
        vec![CURSOR_SET; 5],
        "one SET per entry, 1 ms apart"
    );
    for (r, w) in t.cursors().iter().zip(&want) {
        assert_eq!(r.fd.as_ref().unwrap().2, pixels(w));
        let CursorWant::Image(i) = w else {
            unreachable!()
        };
        assert_eq!((r.cursor()[5], r.cursor()[6]), i.hot(), "guest pixels");
    }
    t.clear();
    for seed in 10..15u8 {
        assert!(t.share.post(image(32, 32, (0, 0), seed)));
    }
    t.tick();
    assert_eq!(t.ops(), vec![CURSOR_SET], "between two entries: one");
}

/// ★ The review of 2026-10-04: the broker sets the grab on EVERY packet (`F_GRABBED`), so a broker
/// that flips it per packet flips the relay's mode per packet — and each return to hover used to
/// make QEMU's main loop define the console's cursor again (allocate up to 256x256, copy, and send
/// it to every VNC client) with nothing pacing it. Here the worst case for the console: a worker
/// that recomposes at once for each mode and a console that shows that frame at once, 1000 flips
/// 1 ms apart — at most one DEFINE per `DEFINE_MIN_MS`, and the newest state once the flips stop.
/// Known-positive: flips spaced by the bound are each defined.
#[test]
fn a_broker_flipping_its_grab_cannot_flood_the_console_with_cursor_defines() {
    let shown = |m: CursorMode| {
        if m == CursorMode::Hover {
            ShownFrame::CursorFree
        } else {
            ShownFrame::CursorComposed
        }
    };
    for spaced in [false, true] {
        let mut t = T::new(true);
        t.up(CAP_CURSOR, 0);
        assert!(t.share.post(image(32, 32, (1, 1), 3)));
        t.tick();
        let mut c = ConsoleCursor::default();
        let flips = if spaced { 20 } else { 1000 };
        let step = if spaced { DEFINE_MIN_MS } else { 1 };
        for i in 0..flips {
            t.pkt(EV_KEY, if i % 2 == 0 { F_GRABBED } else { 0 }, 30);
            t.now += step;
            t.read();
            let u = c.poll(&t.share, Some((40, 40)), shown(t.share.mode()), t.now);
            c.done(u.define.is_some(), u.mouse.is_some());
        }
        if spaced {
            // the first flip is a grab before the console was ever given a cursor: nothing to hide
            assert_eq!(c.defines(), 19, "each later flip, spaced by the bound");
            continue;
        }
        assert!(
            c.defines() <= 1000 / DEFINE_MIN_MS + 2,
            "{} defines for 1000 flips in 1 s",
            c.defines()
        );
        // the flips stop in hover (the last packet had no F_GRABBED): the image, once
        assert_eq!(t.share.mode(), CursorMode::Hover);
        t.now += DEFINE_MIN_MS;
        let u = c.poll(&t.share, Some((40, 40)), ShownFrame::CursorFree, t.now);
        let u2 = c.poll(
            &t.share,
            Some((40, 40)),
            ShownFrame::CursorFree,
            t.now + DEFINE_MIN_MS,
        );
        assert!(
            u.define
                == Some(Some(kf_broker::CursorShape {
                    width: 32,
                    height: 32,
                    hot: (1, 1)
                }))
                || (u.define.is_none() && u2.is_empty()),
            "the newest state, defined once: {u:?} {u2:?}"
        );
    }
}
