// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ The guest's cursor as the host pointer (`OWNER_RULINGS.md` §O, `docs/design/V3_DISPLAY.md`
//! §8.12) — the relay against a scripted link: what reaches the broker, and the mode the worker
//! composes by. Each test has a known-positive half (the same script with the one thing that
//! decides it changed), so a detector that never fires cannot pass.

use kf_broker::console::DEFINE_MIN_MS;
use kf_broker::wire::{
    CAP_CURSOR, CAP_DMABUF, CAP_CURSOR_STREAM, CMD_ATTACH, CMD_COMMIT, CMD_CURSOR, CMD_SIZE, CURSOR_HIDE, CURSOR_SET,
    CURSOR_F_ABS, CURSOR_F_XOR, CURSOR_MOVE, CURSOR_SHOW, EV_ABS, EV_FOCUS, EV_FRAME, EV_GRAB, EV_HELLO, EV_KEY, EV_REL, EV_RELEASE, F_GRABBED, FOURCC_AR24,
    FOURCC_XR24, Pkt,
};
use kf_broker::{
    ConsoleCursor, CursorImage, CursorMode, CursorShare, CursorWant, FrameGeom, FrameRing, Host,
    Link,
    PointerAbs, Recv, Relay, RelayConfig, Sent, ShownFrame, SlotFds, guest_point,
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
    ring: Arc<FrameRing>,
    share: Arc<CursorShare>,
    host: H,
    now: u64,
    serial: u64,
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
            ring.clone(),
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
            ring,
            share: cursor,
            host: H::default(),
            now: 1_000,
            serial: 0,
        }
    }

    /// The worker publishes a frame; `cursor`: the guest's cursor is composed into its pixels. The
    /// width says which (`100 + cursor`), so the ATTACH the broker receives tells it back.
    fn publish(&mut self, cursor: bool) {
        let slot = self.ring.fill_target(None).expect("a fill target");
        self.serial += 1;
        let w = 100 + u32::from(cursor);
        self.ring.describe(
            slot,
            FrameGeom {
                width: w,
                height: 4,
                stride: w * 4,
                fourcc: FOURCC_XR24,
                serial: self.serial,
                cursor,
            },
        );
        self.ring.publish(slot);
    }

    /// The broker presented what it was sent: the credit comes back (`EV_FRAME`).
    fn credit(&mut self) {
        for slot in 0..self.ring.slots() {
            if self.ring.held_mask() & (1 << slot) != 0 {
                let id = self.ring.fds(slot).unwrap().memfd_id();
                let p = Pkt {
                    ty: EV_RELEASE,
                    w0: id as u32,
                    w1: (id >> 32) as u32,
                    ..Pkt::default()
                };
                self.wire.borrow_mut().inbox.extend(p.encode());
            }
        }
        self.pkt(EV_FRAME, 0, 0);
        self.read();
    }

    /// The frames committed to the broker so far, as "composes the cursor" (ATTACH, then COMMIT).
    fn committed(&self) -> Vec<bool> {
        let mut out = Vec::new();
        let mut last = None;
        for r in self.wire.borrow().sent.iter() {
            match r.ty() {
                CMD_ATTACH => last = Some(r.u32_at(4) == 101),
                CMD_COMMIT => out.extend(last),
                _ => {}
            }
        }
        out
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
        // the image/visibility ops; a MOVE is `moves()`'s
        self.cursors()
            .iter()
            .map(|r| r.cursor()[7])
            .filter(|op| *op != CURSOR_MOVE)
            .collect()
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
    let mut last_composed = false;
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
        // the worker's frame follows its want: a cursor kf composes is in the frame the broker
        // is committed (and the host's is hidden then), every other frame is free of it
        let composed = want == CursorWant::Composed;
        if composed || last_composed {
            t.publish(composed);
        }
        last_composed = composed;
        t.tick();
        t.credit();
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

// ── the host cursor follows the frame the broker shows ───────────────────────────────────────

/// ★ H2 (the measured "two cursors"): the worker posts the guest's new cursor at the START of a
/// copy and publishes the frame when it ENDS, and the broker draws a frame only after the previous
/// one was presented. The host cursor used to follow the post, so for that whole time the broker
/// drew the host's arrow beside the I-beam still composed into the frame it showed (and, the other
/// way, no cursor at all). It follows the frame the broker was last committed now.
#[test]
fn the_host_cursor_follows_the_frame_the_broker_shows_never_two_and_never_none() {
    let mut t = T::new(true);
    t.up(CAP_CURSOR, 0);
    let arrow = image(32, 32, (0, 0), 1);
    // an arrow over a cursor-free frame
    assert!(t.share.post(arrow.clone()));
    t.publish(false);
    t.tick();
    assert_eq!(t.ops(), vec![CURSOR_SET]);
    assert_eq!(t.committed(), vec![false]);
    // the guest turns to the XOR I-beam: the post comes first, the frame carrying it later (and the
    // broker, still busy with the last one, has not returned its credit)
    t.clear();
    assert!(t.share.post(CursorWant::Composed));
    t.tick();
    assert_eq!(t.ops(), Vec::<u32>::new(), "the broker still shows the cursor-free frame");
    t.publish(true);
    t.tick();
    assert_eq!(t.ops(), Vec::<u32>::new(), "the frame with the I-beam waits for the credit");
    t.credit();
    assert_eq!(t.committed(), vec![true]);
    assert_eq!(t.ops(), vec![CURSOR_HIDE], "hidden as the frame that carries it is committed");
    // and back to an arrow: the post first again — the I-beam is still in the frame on screen
    t.clear();
    assert!(t.share.post(arrow.clone()));
    t.tick();
    assert_eq!(t.ops(), Vec::<u32>::new(), "no host arrow beside the composed I-beam");
    t.publish(false);
    t.tick();
    t.credit();
    assert_eq!(t.committed(), vec![false]);
    assert_eq!(t.ops(), vec![CURSOR_SHOW], "the held arrow, once the frame is free of the I-beam");
}

/// ★ A cursor the guest hides has nothing in any frame: the host's goes at once (no wait for a
/// frame), and an XOR cursor over frames that never carry it leaves the held image alone.
#[test]
fn a_hidden_cursor_is_hidden_at_once_while_a_composed_one_waits_for_its_frame() {
    let mut t = T::new(true);
    t.up(CAP_CURSOR, 0);
    assert!(t.share.post(image(32, 32, (0, 0), 1)));
    t.publish(false);
    t.tick();
    t.clear();
    assert!(t.share.post(CursorWant::Hidden));
    t.tick();
    assert_eq!(t.ops(), vec![CURSOR_HIDE], "nothing is composed: nothing to wait for");
    t.clear();
    assert!(t.share.post(CursorWant::Composed));
    t.tick();
    assert_eq!(t.ops(), Vec::<u32>::new());
}

// ── the whole path, modelled (worker, relay, console) ────────────────────────────────────────

/// ★ A GPU-free model of everything between the guest's cursor and what the viewers draw, driven by
/// the REAL relay, share, `ConsoleCursor` and `FrameCursors`; only the worker (which needs the GPU)
/// is modelled from `display.rs`: the per-copy decision (`cursor_composed`), the key-deduplicated
/// post (retried when the mailbox is busy), the force on a mode switch, "send when the cursor flag
/// changed" (`NonFlip::wants_send`), and a copy that takes `latency` passes between its start (where
/// the post happens) and its publish.
struct World {
    t: T,
    guest: CursorWant,
    posted: Option<(u8, u64)>,
    mode_seen: CursorMode,
    force: bool,
    sent_flag: Option<bool>,
    /// A copy in flight: (passes left, the cursor flag it composed).
    inflight: Option<(u32, bool)>,
    latency: u32,
    /// The newest published frame carries the composed cursor (None: none yet).
    published: Option<bool>,
    // ── what the broker draws ──
    broker_frame: bool,
    broker_img: Option<u8>,
    broker_visible: bool,
    ops_seen: usize,
    attached: bool,
    // ── what the console draws ──
    cons: ConsoleCursor,
    frames: kf_broker::FrameCursors,
    serial: usize,
    cons_taken: usize,
    cons_frame: ShownFrame,
    /// The console's host cursor: `None` hidden / never defined, `Some(width)` an image.
    cons_img: Option<u32>,
}

impl World {
    fn new(latency: u32) -> World {
        let mut t = T::new(true);
        t.up(CAP_CURSOR, 0);
        t.tick();
        assert_eq!(t.share.mode(), CursorMode::Hover);
        World {
            t,
            guest: CursorWant::Hidden,
            posted: None,
            mode_seen: CursorMode::Off,
            force: false,
            sent_flag: None,
            inflight: None,
            latency,
            published: None,
            broker_frame: false,
            broker_img: None,
            broker_visible: false,
            ops_seen: 0,
            attached: false,
            cons: ConsoleCursor::default(),
            frames: kf_broker::FrameCursors::default(),
            serial: 0,
            cons_taken: 0,
            cons_frame: ShownFrame::Nothing,
            cons_img: None,
        }
    }

    /// The worker's pass: the copy's completion (publish) when it is due, else the next copy's start.
    fn worker(&mut self) {
        let mode = self.t.share.mode();
        if mode != self.mode_seen {
            self.mode_seen = mode;
            self.force = true;
        }
        if let Some((left, flag)) = self.inflight {
            if left > 0 {
                self.inflight = Some((left - 1, flag));
                return;
            }
            self.inflight = None;
            if self.force || self.sent_flag != Some(flag) {
                self.force = false;
                self.sent_flag = Some(flag);
                self.frames.publish(self.serial % 3, flag);
                self.serial += 1;
                self.published = Some(flag);
                self.t.publish(flag);
            }
            return;
        }
        let want = mode.reads().then(|| self.guest.clone());
        let compose = want.as_ref().is_none_or(|w| mode.composes(w));
        if let Some(w) = want {
            let key = w.key();
            if self.posted != Some(key) && self.t.share.post(w) {
                self.posted = Some(key);
            }
        }
        self.inflight = Some((self.latency, compose));
    }

    /// One relay entry (a frame wake, and sometimes the broker handing its credit back); the
    /// commands it sends are what the broker draws, in order.
    fn relay(&mut self, credit: bool) {
        self.t.tick();
        if credit {
            self.t.credit();
        }
        let wire = self.t.wire.borrow();
        for r in &wire.sent[self.ops_seen..] {
            match r.ty() {
                CMD_ATTACH => self.attached = r.u32_at(4) == 101,
                CMD_COMMIT => self.broker_frame = self.attached,
                CMD_CURSOR => match r.cursor()[7] {
                    CURSOR_SET => {
                        self.broker_img = Some(r.fd.as_ref().unwrap().2[0]);
                        self.broker_visible = true;
                    }
                    CURSOR_HIDE => self.broker_visible = false,
                    CURSOR_SHOW => self.broker_visible = true,
                    _ => unreachable!(),
                },
                _ => {}
            }
        }
        self.ops_seen = wire.sent.len();
    }

    /// One console refresh: it takes the newest frame, then polls the cursor.
    fn console(&mut self) {
        if self.serial > self.cons_taken {
            self.cons_taken = self.serial;
            let c = self.frames.took((self.serial - 1) % 3);
            self.cons_frame = if c {
                ShownFrame::CursorComposed
            } else {
                ShownFrame::CursorFree
            };
        }
        self.t.now += 16;
        let u = self
            .cons
            .poll(&self.t.share, Some((40, 40)), self.cons_frame, self.t.now);
        match u.define {
            Some(None) => self.cons_img = None,
            Some(Some(s)) => self.cons_img = Some(s.width),
            None => {}
        }
        self.cons.done(u.define.is_some(), u.mouse.is_some());
    }

    /// The cursors a viewer sees right now: (broker host cursors, broker frame cursors, console
    /// host cursors, console frame cursors).
    fn broker_count(&self) -> usize {
        usize::from(self.broker_visible) + usize::from(self.broker_frame)
    }

    fn console_count(&self) -> usize {
        usize::from(self.cons_img.is_some())
            + usize::from(self.cons_frame == ShownFrame::CursorComposed)
    }

    /// What exactly one cursor of the guest's kind looks like on each viewer.
    fn settled(&self) -> Result<(), String> {
        let (b_frame, b_img, c_frame, c_img) = match &self.guest {
            CursorWant::Hidden => (false, None, false, None),
            CursorWant::Composed => (true, None, true, None),
            CursorWant::Image(i) => (
                false,
                Some(i.pixels()[0]),
                false,
                Some(i.width()),
            ),
        };
        let b = (
            self.broker_frame,
            self.broker_visible.then_some(self.broker_img).flatten(),
        );
        let c = (
            self.cons_frame == ShownFrame::CursorComposed,
            self.cons_img,
        );
        if b != (b_frame, b_img) {
            return Err(format!("broker shows {b:?}, guest {:?}", self.guest));
        }
        if c != (c_frame, c_img) {
            return Err(format!("console shows {c:?}, guest {:?}", self.guest));
        }
        Ok(())
    }
}

fn world_guests() -> [CursorWant; 4] {
    [
        CursorWant::Hidden,
        CursorWant::Composed,
        image(32, 32, (0, 0), 1),
        image(48, 48, (0, 0), 9),
    ]
}

/// A tiny deterministic generator (no dependency).
fn lcg(s: &mut u64) -> u64 {
    *s = s
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *s >> 33
}

/// ★ Convergence: whatever the order in which guest changes, copies, relay entries and console
/// refreshes interleave, once the guest's cursor stops changing every viewer shows exactly that
/// cursor — the composed one iff the shown frame carries it, the host one iff it does not.
#[test]
fn every_viewer_converges_to_the_guests_cursor() {
    let guests = world_guests();
    for latency in 0..4 {
        for seed in 0..300u64 {
            let mut s = seed * 7919 + 1;
            let mut w = World::new(latency);
            for step in 0..200 {
                if lcg(&mut s) % 6 == 0 {
                    w.guest = guests[(lcg(&mut s) % 4) as usize].clone();
                }
                w.worker();
                if lcg(&mut s) % 3 != 0 {
                    let credit = lcg(&mut s) % 2 == 0;
                    w.relay(credit);
                }
                if lcg(&mut s) % 2 == 0 {
                    w.console();
                }
                let _ = step;
            }
            // the guest stops changing; every actor runs at least once per pass
            for _ in 0..(8 + 2 * latency) {
                w.worker();
                w.relay(true);
                w.console();
            }
            if let Err(e) = w.settled() {
                panic!("latency {latency} seed {seed}: {e}");
            }
        }
    }
}

/// ★ "One cursor, never two": at every instant a viewer shows at most one of {the host cursor, the
/// composed one}. A cursor that changes kind passes through the old frame, so the broker gets
/// TWO cursors for as long as a copy takes if the host cursor is changed before the frame is.
#[test]
fn a_viewer_never_shows_two_cursors() {
    let guests = world_guests();
    let mut worst = (0usize, 0usize);
    let mut first = None;
    for latency in 0..4 {
        for seed in 0..300u64 {
            let mut s = seed * 104729 + 3;
            let mut w = World::new(latency);
            for _ in 0..200 {
                if lcg(&mut s) % 6 == 0 {
                    w.guest = guests[(lcg(&mut s) % 4) as usize].clone();
                }
                w.worker();
                if lcg(&mut s) % 3 != 0 {
                    let credit = lcg(&mut s) % 2 == 0;
                    w.relay(credit);
                }
                if lcg(&mut s) % 2 == 0 {
                    w.console();
                }
                worst = (worst.0.max(w.broker_count()), worst.1.max(w.console_count()));
                if (w.broker_count() > 1 || w.console_count() > 1) && first.is_none() {
                    first = Some(format!(
                        "latency {latency} seed {seed}: broker {} (frame {}, host {}), console {} (frame {:?}, host {:?}), guest {:?}",
                        w.broker_count(),
                        w.broker_frame,
                        w.broker_visible,
                        w.console_count(),
                        w.cons_frame,
                        w.cons_img,
                        w.guest
                    ));
                }
            }
        }
    }
    assert!(first.is_none(), "{first:?} (worst {worst:?})");
}

// ── the broker draws the guest's cursor itself (CAP_CURSOR_STREAM) ───────────────────────────

/// A `w` x `h` XOR inversion mask: alpha 255 and the XOR colour where `on`, zero elsewhere.
fn xor_mask(w: u32, h: u32, on: &[(u32, u32)], hot: (u32, u32)) -> CursorWant {
    let mut px = vec![0u8; (w * h * 4) as usize];
    for &(x, y) in on {
        px[((y * w + x) * 4) as usize..][..4].copy_from_slice(&[255, 255, 255, 255]);
    }
    CursorWant::Image(Arc::new(CursorImage::new(w, h, hot, px).unwrap().into_xor()))
}

/// Every MOVE sent, in order: (the image's top-left, the absolute pointer pair if the flag is set).
fn moves(t: &T) -> Vec<((i32, i32), Option<(i32, i32)>)> {
    t.cursors()
        .iter()
        .filter(|r| r.cursor()[7] == CURSOR_MOVE)
        .map(|r| {
            let f = u16::from_le_bytes([r.raw[2], r.raw[3]]);
            let c = r.cursor();
            // width/height are zero unless the flag says they are the pointer
            assert!(f == 0 || f == CURSOR_F_ABS, "flags {f}");
            assert!(f == CURSOR_F_ABS || (c[0], c[1]) == (0, 0), "a pair without its flag");
            (
                (c[5] as i32, c[6] as i32),
                (f == CURSOR_F_ABS).then_some((c[0] as i32, c[1] as i32)),
            )
        })
        .collect()
}

fn abs(t: &T, x: i32, y: i32, w: u32, h: u32) {
    let p = Pkt {
        ty: EV_ABS,
        x,
        y,
        w0: w,
        w1: h,
        ..Pkt::default()
    };
    t.wire.borrow_mut().inbox.extend(p.encode());
}

fn grab(t: &mut T, on: bool) {
    t.pkt(EV_KEY, if on { F_GRABBED } else { 0 }, 30);
    t.read();
}

/// ★ The stream is negotiated by the HELLO bit alone: with it the worker is told nothing is composed
/// (`CursorShare::stream`); without it (an old broker) the composing path is untouched.
#[test]
fn the_stream_is_the_brokers_yes_and_an_old_broker_keeps_the_composing_path() {
    let mut t = T::new(true);
    t.up(CAP_CURSOR, 0);
    t.tick();
    assert!(!t.share.stream(), "no bit: composed as before");
    let mut t = T::new(true);
    t.up(CAP_CURSOR | CAP_CURSOR_STREAM, 0);
    t.tick();
    assert!(t.share.stream());
    // the bit without CAP_CURSOR is nothing (the cursor path is off altogether)
    let mut t = T::new(true);
    t.up(CAP_CURSOR_STREAM, 0);
    t.tick();
    assert!(!t.share.stream());
    // the broker goes: the worker composes again
    let mut t = T::new(true);
    t.up(CAP_CURSOR | CAP_CURSOR_STREAM, 0);
    t.tick();
    t.wire.borrow_mut().eof = true;
    t.read();
    t.tick();
    assert!(!t.share.stream());
}

/// ★ Streaming: the image goes as today (SET once, HIDE for nothing, SHOW for the held one),
/// a cursor only a frame could show is HIDE (nothing composes), and an XOR mask is a SET flagged `CURSOR_F_XOR`, with the mask's bytes.
#[test]
fn hover_streams_the_image_and_the_xor_mask() {
    let mut t = T::new(true);
    t.up(CAP_CURSOR | CAP_CURSOR_STREAM, 0);
    t.tick();
    t.share.note_point(Some((10, 20)));
    let a = image(32, 32, (0, 0), 1);
    for (want, op) in [
        (a.clone(), Some(CURSOR_SET)),
        (CursorWant::Composed, Some(CURSOR_HIDE)),
        (a.clone(), Some(CURSOR_SHOW)),
        (CursorWant::Hidden, Some(CURSOR_HIDE)),
        (CursorWant::Hidden, None),
    ] {
        t.clear();
        assert!(t.share.post(want.clone()));
        t.share.note_point(Some((10 + t.now as i32 % 7, 20)));
        t.tick();
        assert_eq!(t.ops(), op.into_iter().collect::<Vec<_>>(), "{want:?}");
    }
    // the XOR mask
    t.clear();
    let m = xor_mask(16, 16, &[(3, 3), (4, 3)], (1, 2));
    assert!(t.share.post(m.clone()));
    t.tick();
    let recs: Vec<Rec> = t.cursors().into_iter().filter(|r| r.cursor()[7] != CURSOR_MOVE).collect();
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].cursor()[7], CURSOR_SET);
    assert_eq!(&recs[0].raw[2..4], &CURSOR_F_XOR.to_le_bytes(), "flags = F_XOR");
    assert_eq!((recs[0].cursor()[5], recs[0].cursor()[6]), (1, 2), "the hot spot");
    assert_eq!(recs[0].fd.as_ref().unwrap().2, pixels(&m), "the mask's bytes");
    // a plain image after it: no flag
    t.clear();
    assert!(t.share.post(image(32, 32, (0, 0), 5)));
    t.tick();
    assert_eq!(&t.cursors()[0].raw[2..4], &[0, 0]);
    // the same pixels as a plain image are another cursor
    let same = CursorWant::Image(Arc::new(CursorImage::new(16, 16, (1, 2), pixels(&m)).unwrap()));
    assert_ne!(same.key(), m.key(), "the digest covers the XOR bit");
}

/// ★ MODE-FREE: kayfabe reports, the viewer decides (grab is the viewer's state, not the protocol's).
/// MOVE is ONE atomic pair — the image's top-left (the worker's note) and the last ABSOLUTE pointer
/// the relay injected, mapped to guest frame pixels (the committed frame's size), flag clear after a
/// relative event or before any injection — sent in hover AND grab, once per change of the pair,
/// never per input event; forgotten nowhere (no mode). Clamped; none while hidden.
#[test]
fn move_is_one_atomic_pair_sent_once_per_change_in_every_mode() {
    let mut t = T::new(true);
    t.up(CAP_CURSOR | CAP_CURSOR_STREAM, 0);
    t.tick();
    // a committed 100x4 frame: the guest's frame size, for the pointer's mapping
    t.publish(false);
    t.tick();
    assert!(t.share.post(image(32, 32, (0, 0), 1)));
    t.share.note_point(Some((5, 6)));
    t.tick();
    assert_eq!(moves(&t), vec![((5, 6), None)], "no absolute input yet: the flag is clear");
    // absolute input (hover): the pair appears, mapped through QEMU's tablet scaling to guest pixels
    abs(&t, 50, 2, 200, 8);
    t.read();
    let g = guest_point((50, 2), (200, 8), (100, 4)).unwrap();
    assert_eq!(g, (24, 0), "50/200 of 100 px through the two truncations");
    assert_eq!(moves(&t).last(), Some(&((5, 6), Some(g))));
    let n = moves(&t).len();
    t.tick();
    t.tick();
    assert_eq!(moves(&t).len(), n, "the same pair is not sent twice");
    // the cursor moves, or the pointer moves: one more each, whatever the number of events between
    for i in 0..30 {
        t.share.note_point(Some((9 + i, 9)));
    }
    t.tick();
    assert_eq!(moves(&t).len(), n + 1);
    assert_eq!(moves(&t).last(), Some(&((38, 9), Some(g))), "the newest of many notes");
    abs(&t, 150, 6, 200, 8);
    t.read();
    let g2 = guest_point((150, 6), (200, 8), (100, 4)).unwrap();
    assert_eq!(moves(&t).last(), Some(&((38, 9), Some(g2))));
    // GRAB: relative input clears the flag in the very next pair (the viewer then draws at the top-left)
    grab(&mut t, true);
    t.pkt(EV_REL, 0, 3);
    t.read();
    t.share.note_point(Some((40, 10)));
    t.tick();
    assert_eq!(moves(&t).last(), Some(&((40, 10), None)), "relative: flag clear, width/height zero");
    // ... and it stays clear through grab-time cursor moves
    t.share.note_point(Some((41, 11)));
    t.tick();
    assert_eq!(moves(&t).last(), Some(&((41, 11), None)));
    // the grab ends: the relay puts the absolute device back where the host pointer was
    grab(&mut t, false);
    t.pkt(EV_GRAB, 0, 0);
    t.read();
    t.tick();
    assert_eq!(moves(&t).last(), Some(&((41, 11), Some(g2))), "absolute again");
    // far outside the frame: clamped to -256..=8192 (the pointer is clamped too)
    t.share.note_point(Some((i32::MIN, 100_000_000)));
    t.tick();
    assert_eq!(moves(&t).last(), Some(&((-256, 8192), Some(g2))));
    // hidden: nothing to place; the guest's cursor back: SHOW, then the pair if it changed
    t.clear();
    assert!(t.share.post(CursorWant::Hidden));
    t.share.note_point(Some((7, 7)));
    t.tick();
    assert_eq!(t.ops(), vec![CURSOR_HIDE]);
    assert_eq!(moves(&t), vec![], "hidden: no MOVE");
    t.clear();
    assert!(t.share.post(image(32, 32, (0, 0), 1)));
    t.tick();
    assert_eq!(t.ops(), vec![CURSOR_SHOW]);
    assert_eq!(moves(&t), vec![((7, 7), Some(g2))]);
    // no point known (no enabled cursor): no MOVE
    t.clear();
    t.share.note_point(None);
    t.tick();
    assert_eq!(moves(&t), vec![]);
    assert!(t.relay.counters().cursor_moves >= 6);
}

/// The VMM-neutral mapping of the broker's pointer to the guest's pixels is the one
/// `hot_from_pointer` uses (its two truncating scalings).
#[test]
fn the_pointer_maps_to_guest_pixels_through_the_two_truncating_scalings() {
    assert_eq!(guest_point((0, 0), (1024, 768), (1920, 1080)), Some((0, 0)));
    // 48 of 1024 reaches the guest as 47 of 1024 (brkA4's measurement), on a 1024-wide head
    assert_eq!(guest_point((48, 8), (1024, 695), (1024, 695)).map(|p| p.0), Some(47));
    assert_eq!(guest_point((8, 8), (695, 695), (695, 695)).map(|p| p.0), Some(7));
    assert_eq!(guest_point((1, 1), (0, 5), (10, 10)), None, "empty range");
    assert_eq!(guest_point((1, 1), (5, 5), (0, 10)), None, "empty frame");
    assert_eq!(guest_point((-5, -5), (100, 100), (100, 100)), Some((0, 0)), "negative pins to 0");
    let far = guest_point((i32::MAX, i32::MAX), (1, 1), (8192, 8192));
    assert!(far.is_none() || far.unwrap().0 >= 0, "no overflow");
}

/// ★ The known-negative of the above: an old broker (no stream bit) is never sent a MOVE or the XOR
/// flag, in grab or in hover — the grab sends nothing at all, as before.
#[test]
fn a_broker_without_the_stream_is_never_sent_a_move_or_the_xor_flag() {
    let mut t = T::new(true);
    t.up(CAP_CURSOR, 0);
    t.tick();
    assert!(t.share.post(image(32, 32, (0, 0), 1)));
    t.share.note_point(Some((5, 6)));
    t.tick();
    grab(&mut t, true);
    t.share.note_point(Some((50, 60)));
    t.tick();
    assert_eq!(moves(&t), vec![]);
    assert!(t.share.post(xor_mask(16, 16, &[(3, 3)], (1, 2))));
    grab(&mut t, false);
    t.tick();
    for r in t.cursors() {
        assert_eq!(&r.raw[2..4], &[0, 0], "no flags");
    }
}

/// ★ The console's cursor sink is not used while the broker streams: the frames carry no cursor and
/// the console defines none (and hides one it was given before the stream began).
#[test]
fn the_console_defines_no_cursor_while_the_broker_streams() {
    let mut t = T::new(true);
    t.up(CAP_CURSOR, 0);
    t.tick();
    assert!(t.share.post(image(8, 8, (0, 0), 1)));
    let mut c = ConsoleCursor::default();
    let u = c.poll(&t.share, Some((0, 0)), ShownFrame::CursorFree, t.now);
    assert!(u.define.is_some(), "composing path, hover: the console shows the image");
    c.done(true, true);
    // a streaming broker takes over
    let mut t2 = T::new(true);
    t2.up(CAP_CURSOR | CAP_CURSOR_STREAM, 0);
    t2.tick();
    assert!(t2.share.post(image(8, 8, (0, 0), 1)));
    let mut c2 = ConsoleCursor::default();
    let u = c2.poll(&t2.share, Some((0, 0)), ShownFrame::CursorFree, t2.now);
    assert_eq!(u.define, None, "never defined");
    // and one defined earlier is hidden once the stream starts
    t.wire.borrow_mut().eof = true;
    t.read();
    let mut t3 = T::new(true);
    t3.up(CAP_CURSOR, 0);
    t3.tick();
    assert!(t3.share.post(image(8, 8, (0, 0), 1)));
    let mut c3 = ConsoleCursor::default();
    let u = c3.poll(&t3.share, Some((0, 0)), ShownFrame::CursorFree, t3.now);
    c3.done(u.define.is_some(), true);
    t3.share.set_stream(true);
    let u = c3.poll(&t3.share, Some((0, 0)), ShownFrame::CursorFree, t3.now + 100);
    assert_eq!(u.define, Some(None), "hidden");
}
