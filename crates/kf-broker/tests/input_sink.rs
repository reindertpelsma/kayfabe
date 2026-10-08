// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ The input policy behind the VMM-neutral [`InputSink`] (`OWNER_RULINGS.md` §V, 2026-10-08;
//! `docs/design/V3_DISPLAY.md` §8.20): broker packets through the REAL relay (a scripted link)
//! into [`InputPolicy`], against two sinks — a FAKE that records each verb (and scales an absolute
//! position the way QEMU does), and an EVDEV-style recorder that turns the verbs into the
//! `(type, code, value)` stream a virtio-input device would carry — to show the trait is not
//! QEMU-shaped.

use kf_broker::input::{MAX_POINTER_DEVICES, MAX_SINK_CALLS_PER_INPUT};
use kf_broker::wire::{
    CAP_DMABUF, CMD_SIZE, EV_ABS, EV_BTN, EV_CLOSE, EV_GRAB, EV_HELLO, EV_KEY, EV_REL, EV_SURFACE,
    EV_WHEEL, F_GRABBED, PKT_SIZE, Pkt,
};
use kf_broker::{
    AbsRange, Button, FrameRing, Host, InputPolicy, InputSink, Link, Pointer, PointerDevice,
    PowerRequest, Recv, Relay, RelayConfig, Sent,
};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

// ── a scripted link: the broker's bytes, nothing sent back that matters here ────────────────

#[derive(Default)]
struct Wire {
    inbox: VecDeque<u8>,
    /// The broker went away: the next read past the inbox sees the end of the stream.
    eof: bool,
}

#[derive(Clone)]
struct Fake(Rc<RefCell<Wire>>);

struct Sock;

impl Link for Fake {
    type Sock = Sock;
    fn connect(&mut self, _: &Path) -> Result<Sock, String> {
        Ok(Sock)
    }
    fn fd(&self, _: &Sock) -> i32 {
        77
    }
    fn peer_uid(&mut self, _: &Sock) -> Result<u32, String> {
        Ok(1000)
    }
    fn effective_uid(&mut self) -> u32 {
        1000
    }
    fn send(&mut self, _: &Sock, _: &[u8; CMD_SIZE], _: Option<BorrowedFd<'_>>) -> Sent {
        Sent::Done
    }
    fn recv(&mut self, _: &Sock, buf: &mut [u8]) -> Recv {
        let mut w = self.0.borrow_mut();
        if w.inbox.is_empty() {
            return if std::mem::take(&mut w.eof) {
                Recv::Closed
            } else {
                Recv::Empty
            };
        }
        let n = buf.len().min(w.inbox.len());
        for b in buf.iter_mut().take(n) {
            *b = w.inbox.pop_front().unwrap();
        }
        Recv::Bytes(n)
    }
}

#[derive(Default)]
struct NoHost;

impl Host for NoHost {
    fn watch(&mut self, _: i32, _: bool, _: bool) {}
    fn timer(&mut self, _: Option<u64>) {}
}

// ── sink 1: the fake, one record per verb ───────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Key(u16, bool),
    Button(Button, bool, Pointer),
    Wheel(i32, i32, Pointer),
    /// The position, its range, and where it lands on a 0..=32767 axis (QEMU's scale).
    Abs(u32, u32, AbsRange, (u32, u32)),
    Rel(i32, i32),
    Sync,
    Devices,
    Select(u32, Pointer),
    Missing(Pointer),
    Close(PowerRequest),
    Resize(u32, u32, u32),
}

#[derive(Default)]
struct FakeSink {
    calls: Vec<Call>,
    devices: Vec<PointerDevice>,
    /// Key codes the "VMM" has no mapping for.
    unmapped: Vec<u16>,
}

impl InputSink for FakeSink {
    fn key(&mut self, code: u16, down: bool) -> bool {
        self.calls.push(Call::Key(code, down));
        !self.unmapped.contains(&code)
    }
    fn button(&mut self, b: Button, down: bool, to: Pointer) {
        self.calls.push(Call::Button(b, down, to));
    }
    fn wheel(&mut self, dx: i32, dy: i32, to: Pointer) {
        self.calls.push(Call::Wheel(dx, dy, to));
    }
    fn abs(&mut self, x: u32, y: u32, r: AbsRange) {
        assert!(x < r.width && y < r.height, "abs ({x}, {y}) outside {r:?}");
        self.calls.push(Call::Abs(x, y, r, r.scale(x, y, 32767)));
    }
    fn rel(&mut self, dx: i32, dy: i32) {
        self.calls.push(Call::Rel(dx, dy));
    }
    fn sync(&mut self) {
        self.calls.push(Call::Sync);
    }
    fn pointer_devices(&mut self, out: &mut [PointerDevice]) -> usize {
        self.calls.push(Call::Devices);
        assert_eq!(
            out.len(),
            MAX_POINTER_DEVICES,
            "the policy asks for a bounded list"
        );
        // a VMM with more devices than it is asked for fills the slice only
        let n = self.devices.len().min(out.len());
        out[..n].clone_from_slice(&self.devices[..n]);
        n
    }
    fn select_pointer(&mut self, id: u32, kind: Pointer) {
        self.calls.push(Call::Select(id, kind));
    }
    fn missing_pointer(&mut self, kind: Pointer) {
        self.calls.push(Call::Missing(kind));
    }
    fn close(&mut self, r: PowerRequest) {
        self.calls.push(Call::Close(r));
    }
    fn resize_hint(&mut self, w: u32, h: u32, mhz: u32) {
        self.calls.push(Call::Resize(w, h, mhz));
    }
}

fn dev(id: u32, kind: Pointer, paravirtual: bool, name: &str) -> PointerDevice {
    PointerDevice {
        id,
        kind,
        paravirtual,
        name: name.into(),
    }
}

/// QEMU's q35 list as `query-mice` reports it with the launcher's devices: the PS/2 mouse
/// (relative, not paravirtual), the virtio tablet (absolute) and the virtio mouse (relative).
fn q35_devices() -> Vec<PointerDevice> {
    vec![
        dev(2, Pointer::Relative, false, "QEMU PS/2 Mouse"),
        dev(4, Pointer::Absolute, true, "QEMU Virtio Tablet"),
        dev(5, Pointer::Relative, true, "QEMU Virtio Mouse"),
    ]
}

// ── the harness: relay → policy → sink ──────────────────────────────────────────────────────

struct T<S: InputSink> {
    wire: Rc<RefCell<Wire>>,
    relay: Relay<Fake>,
    policy: InputPolicy,
    sink: S,
    host: NoHost,
    now: u64,
}

impl<S: InputSink> T<S> {
    fn new(sink: S) -> T<S> {
        let wire = Rc::new(RefCell::new(Wire::default()));
        let ring = Arc::new(FrameRing::new(kf_broker::slots::BROKER_SLOTS, true));
        let relay = Relay::new(
            RelayConfig {
                path: PathBuf::from("/run/test/display.sock"),
                extra_uid: None,
            },
            ring,
            Fake(wire.clone()),
        );
        let mut t = T {
            wire,
            relay,
            policy: InputPolicy::new(),
            sink,
            host: NoHost,
            now: 1_000,
        };
        t.relay.start(t.now, &mut t.host);
        t.relay.on_timer(t.now, &mut t.host);
        t.raw(&Pkt {
            ty: EV_HELLO,
            w0: 2,
            w1: CAP_DMABUF,
            ..Pkt::default()
        });
        t.read();
        assert!(t.relay.active());
        t
    }

    /// The broker restarts: the stream ends, the relay retries, a new HELLO.
    fn reconnect(&mut self) {
        if self.relay.connected() {
            self.wire.borrow_mut().eof = true;
            self.read();
        }
        self.now += 10_000;
        self.relay.on_timer(self.now, &mut self.host);
        self.pkt(EV_HELLO, 0, 0, 2, CAP_DMABUF);
        self.read();
    }

    fn raw(&self, p: &Pkt) {
        self.wire.borrow_mut().inbox.extend(p.encode());
    }

    fn pkt(&self, ty: u16, x: i32, y: i32, w0: u32, w1: u32) {
        self.raw(&Pkt {
            ty,
            x,
            y,
            w0,
            w1,
            ..Pkt::default()
        });
    }

    /// As the broker sends every packet while grabbed: with `F_GRABBED`.
    fn grabbed(&self, ty: u16, x: i32, y: i32, w0: u32, w1: u32) {
        self.raw(&Pkt {
            ty,
            flags: F_GRABBED,
            x,
            y,
            w0,
            w1,
            ..Pkt::default()
        });
    }

    /// What the VMM's loop does on readability (`kf-qemu`'s `BrokerSeat::ready`): read one batch,
    /// the first-connection device check, then deliver. Returns the inputs the relay emitted.
    fn read(&mut self) -> usize {
        let mut out = Vec::new();
        self.relay.on_socket(
            self.now,
            true,
            false,
            &mut self.host,
            &mut out,
            kf_broker::conn::READ_BATCH,
        );
        if self.relay.connected() {
            self.policy.connected(&mut self.sink);
        }
        self.policy.deliver(&out, &mut self.sink);
        out.len()
    }
}

impl T<FakeSink> {
    fn fake() -> T<FakeSink> {
        T::new(FakeSink {
            devices: q35_devices(),
            ..FakeSink::default()
        })
    }

    fn take(&mut self) -> Vec<Call> {
        std::mem::take(&mut self.sink.calls)
    }
}

const R1080: AbsRange = AbsRange {
    width: 1920,
    height: 1080,
};

// ── the deliverables ────────────────────────────────────────────────────────────────────────

/// An absolute position lands at its place scaled by the range — QEMU's `value * 32767 / range`
/// (measured on the guest as evtest ABS_X 1706 for x = 100 of 1920, §8.17) — clamped into the
/// range, and a zero range is dropped; one sync per position.
#[test]
fn absolute_lands_at_the_right_scaled_place() {
    let mut t = T::fake();
    t.take();
    t.pkt(EV_ABS, 100, 100, 1920, 1080);
    t.pkt(EV_ABS, 1919, 1079, 1920, 1080);
    t.pkt(EV_ABS, 5000, -7, 1920, 1080);
    t.pkt(EV_ABS, 5, 5, 0, 1080);
    t.pkt(EV_ABS, 640, 360, 1280, 720);
    t.read();
    assert_eq!(
        t.take(),
        vec![
            Call::Abs(100, 100, R1080, (1706, 3033)),
            Call::Sync,
            Call::Abs(1919, 1079, R1080, (32749, 32736)),
            Call::Sync,
            Call::Abs(1919, 0, R1080, (32749, 0)),
            Call::Sync,
            Call::Abs(
                640,
                360,
                AbsRange {
                    width: 1280,
                    height: 720
                },
                (16383, 16383)
            ),
            Call::Sync,
        ]
    );
    assert_eq!(t.policy.counters().abs, 4);
}

/// Relative deltas of one read batch are summed (saturating) into ONE report.
#[test]
fn relative_deltas_sum_with_saturation() {
    let mut t = T::fake();
    t.grabbed(EV_GRAB, 1, 0, 0, 0);
    t.read();
    t.take();
    for _ in 0..16 {
        t.grabbed(EV_REL, 4, 2, 0, 0);
    }
    t.read();
    assert_eq!(t.take(), vec![Call::Rel(64, 32), Call::Sync]);
    t.grabbed(EV_REL, i32::MAX, i32::MIN, 0, 0);
    t.grabbed(EV_REL, 7, -7, 0, 0);
    t.grabbed(EV_REL, 1, 1, 0, 0);
    t.read();
    assert_eq!(
        t.take(),
        vec![Call::Rel(i32::MAX, i32::MIN + 1), Call::Sync],
        "saturating at each step, as the relay sums"
    );
}

/// Key edges keep their order (a chord with modifiers lands as sent); a code that is not an
/// evdev key — `KEY_RESERVED`, a BUTTON code, past `KEY_MAX`, negative — is refused before the
/// VMM sees it; a code the VMM cannot map is counted.
#[test]
fn keys_with_modifiers_in_order_and_unknown_codes_refused() {
    let mut t = T::fake();
    t.sink.unmapped = vec![0x2fe];
    t.take();
    let log = kf_broker::capture_log();
    // shift+a, ctrl+l (KEY_LEFTSHIFT 42, KEY_A 30, KEY_LEFTCTRL 29, KEY_L 38)
    for (c, d) in [
        (42, 1),
        (30, 1),
        (30, 0),
        (42, 0),
        (29, 1),
        (38, 1),
        (38, 0),
        (29, 0),
    ] {
        t.pkt(EV_KEY, c, d, 0, 0);
    }
    for bad in [0, 0x110, 0x100, 0x220, 0x2c0, 0x300, -1, i32::MAX, i32::MIN] {
        t.pkt(EV_KEY, bad, 1, 0, 0);
    }
    t.pkt(EV_KEY, 0x2fe, 1, 0, 0);
    t.read();
    assert_eq!(
        t.take(),
        vec![
            Call::Key(42, true),
            Call::Key(30, true),
            Call::Key(30, false),
            Call::Key(42, false),
            Call::Key(29, true),
            Call::Key(38, true),
            Call::Key(38, false),
            Call::Key(29, false),
            Call::Key(0x2fe, true),
        ],
        "no sync after a key: the VMM's key verb is a whole report"
    );
    let c = t.policy.counters();
    // -1, 0x300 and the two extremes are filtered by the relay already (evdev's range)
    assert_eq!((c.keys, c.keys_refused, c.keys_unmapped), (8, 5, 1));
    let l = log.lines();
    assert_eq!(
        l.iter()
            .filter(|l| l.contains("is not an evdev key code"))
            .count(),
        1,
        "one line per kind of refusal, not per packet: {l:?}"
    );
}

/// In hover buttons and the wheel go to the ABSOLUTE device; under grab to the RELATIVE one, and
/// no ABS reaches the VMM (dropped, counted by the relay); unforwarded button codes are dropped.
#[test]
fn buttons_and_wheel_route_in_hover_and_in_grab() {
    let mut t = T::fake();
    t.take();
    t.pkt(EV_BTN, 0x110, 1, 0, 0);
    t.pkt(EV_BTN, 0x110, 0, 0, 0);
    t.pkt(EV_BTN, 0x117, 1, 0, 0); // BTN_TASK: not forwarded
    t.pkt(EV_WHEEL, 1, 0, 0, 0);
    t.pkt(EV_WHEEL, 0, -3, 0, 0);
    t.read();
    let a = Pointer::Absolute;
    assert_eq!(
        t.take(),
        vec![
            Call::Button(Button::Left, true, a),
            Call::Sync,
            Call::Button(Button::Left, false, a),
            Call::Sync,
            Call::Wheel(0, 1, a),
            Call::Sync,
            Call::Wheel(-1, 0, a),
            Call::Sync,
        ]
    );
    assert_eq!(t.policy.counters().buttons_dropped, 1);
    let log = kf_broker::capture_log();
    t.grabbed(EV_GRAB, 1, 0, 0, 0);
    t.grabbed(EV_BTN, 0x111, 1, 0, 0);
    t.grabbed(EV_ABS, 10, 10, 1920, 1080);
    t.grabbed(EV_WHEEL, -2, 0, 0, 0);
    t.grabbed(EV_BTN, 0x111, 0, 0, 0);
    t.grabbed(EV_BTN, 0x113, 1, 0, 0);
    t.read();
    let r = Pointer::Relative;
    assert_eq!(
        t.take(),
        vec![
            Call::Devices,
            Call::Select(5, r),
            Call::Button(Button::Right, true, r),
            Call::Sync,
            Call::Wheel(0, -1, r),
            Call::Sync,
            Call::Button(Button::Right, false, r),
            Call::Sync,
            Call::Button(Button::Side, true, r),
            Call::Sync,
        ],
        "no Abs while grabbed"
    );
    t.now += 1_000;
    t.grabbed(EV_REL, 1, 0, 0, 0);
    t.read();
    assert!(
        log.lines()
            .iter()
            .any(|l| l.contains("1 absolute report(s) dropped")),
        "{:?}",
        log.lines()
    );
}

/// The grab picks the PARAVIRTUAL relative device over an earlier emulated one (the PS/2 mouse
/// q35 lists first), the release the absolute one and re-syncs it to the last pre-grab position
/// (where the host's locked pointer is); with no paravirtual device of the kind, the first.
#[test]
fn grab_switch_and_release_resync() {
    let mut t = T::fake();
    t.pkt(EV_ABS, 300, 200, 1920, 1080);
    t.read();
    t.take();
    let log = kf_broker::capture_log();
    t.grabbed(EV_GRAB, 1, 0, 0, 0);
    t.grabbed(EV_REL, 9, 9, 0, 0);
    t.read();
    t.pkt(EV_GRAB, 0, 0, 0, 0);
    t.read();
    assert_eq!(
        t.take(),
        vec![
            Call::Devices,
            Call::Select(5, Pointer::Relative),
            Call::Rel(9, 9),
            Call::Sync,
            Call::Devices,
            Call::Select(4, Pointer::Absolute),
            Call::Abs(300, 200, R1080, (5119, 6067)),
            Call::Sync,
        ]
    );
    let l = log.lines();
    assert!(
        l.iter()
            .any(|l| l == "pointing device -> #5 QEMU Virtio Mouse (relative)")
            && l.iter()
                .any(|l| l == "pointing device -> #4 QEMU Virtio Tablet (absolute)"),
        "the line input_proof.sh greps for: {l:?}"
    );
    // no paravirtual relative device: the first relative one
    t.sink.devices = vec![
        dev(4, Pointer::Absolute, true, "QEMU Virtio Tablet"),
        dev(2, Pointer::Relative, false, "QEMU PS/2 Mouse"),
        dev(3, Pointer::Relative, false, "QEMU USB Mouse"),
    ];
    t.grabbed(EV_GRAB, 1, 0, 0, 0);
    t.read();
    assert_eq!(
        t.take(),
        vec![Call::Devices, Call::Select(2, Pointer::Relative)]
    );
    assert_eq!(t.policy.counters().switches, 3);
}

/// Without an absolute device the policy warns ONCE (at the first connection), and the VMM is
/// told once so it can say how to add one; the ungrab that cannot put the pointer back does not
/// warn again. A missing relative device is warned once too, at the first grab.
#[test]
fn the_missing_absolute_device_is_warned_once() {
    let log = kf_broker::capture_log();
    let mut t = T::new(FakeSink {
        devices: vec![dev(2, Pointer::Relative, false, "QEMU PS/2 Mouse")],
        ..FakeSink::default()
    });
    assert_eq!(
        t.take(),
        vec![Call::Devices, Call::Missing(Pointer::Absolute)],
        "checked at the first connection"
    );
    t.pkt(EV_KEY, 30, 1, 0, 0);
    t.read();
    t.grabbed(EV_GRAB, 1, 0, 0, 0);
    t.read();
    t.pkt(EV_GRAB, 0, 0, 0, 0);
    t.read();
    t.grabbed(EV_GRAB, 1, 0, 0, 0);
    t.read();
    t.pkt(EV_GRAB, 0, 0, 0, 0);
    t.read();
    let calls = t.take();
    assert_eq!(
        calls
            .iter()
            .filter(|c| matches!(c, Call::Missing(_)))
            .count(),
        0,
        "{calls:?}"
    );
    let warn = |l: &Vec<String>| {
        l.iter()
            .filter(|l| l.starts_with("NO absolute pointing device exists"))
            .count()
    };
    assert_eq!(warn(&log.lines()), 1, "{:?}", log.lines());
    // a second connection (a broker restart) does not check again
    let mut t2 = T::new(FakeSink::default());
    let calls = t2.take();
    assert_eq!(
        calls,
        vec![Call::Devices, Call::Missing(Pointer::Absolute)],
        "a NEW policy checks; the same one never twice"
    );
    t2.reconnect();
    assert!(t2.relay.active() && t2.relay.counters().reconnects == 1);
    t2.grabbed(EV_GRAB, 1, 0, 0, 0);
    t2.read();
    t2.grabbed(EV_GRAB, 1, 0, 0, 0);
    t2.read();
    assert_eq!(
        t2.take(),
        vec![
            Call::Devices,
            Call::Missing(Pointer::Relative),
            Call::Devices
        ],
        "relative: warned at the first grab only"
    );
}

/// CLOSE: force → stop now, else a powerdown the guest decides on; SURFACE → a resize hint.
#[test]
fn close_and_surface_are_one_verb_each() {
    let mut t = T::fake();
    t.take();
    t.pkt(EV_CLOSE, 0, 0, 0, 0);
    t.pkt(EV_CLOSE, 1, 0, 0, 0);
    t.pkt(EV_SURFACE, 1600, 900, 59_940, 0);
    t.read();
    assert_eq!(
        t.take(),
        vec![
            Call::Close(PowerRequest::Powerdown),
            Call::Close(PowerRequest::ForceOff),
            Call::Resize(1600, 900, 59_940),
        ]
    );
}

/// A little deterministic generator (no dependency).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn i32(&mut self) -> i32 {
        const EDGES: [i32; 8] = [
            0,
            1,
            -1,
            i32::MAX,
            i32::MIN,
            0x7fc0_0000,
            -0x0040_0000,
            1 << 20,
        ];
        let r = self.next();
        if r & 3 == 0 {
            EDGES[(r >> 2) as usize % EDGES.len()]
        } else if r & 3 == 1 {
            // around evdev's codes and a window's size
            ((r >> 32) % 0x320) as i32
        } else {
            (r >> 32) as i32
        }
    }
}

/// ★ Hostile input: random types (the input ones weighted up), extreme and NaN-like operands
/// (`0x7fc00000` is a float NaN's bits), random flags flipping the grab, a VMM listing more
/// devices than asked for, and a truncated packet at the end — never a panic, every absolute
/// position inside its range (the fake asserts it), every key code an evdev key, and never more
/// than [`MAX_SINK_CALLS_PER_INPUT`] × (inputs) sink calls, the relay emitting at most two
/// inputs per packet and at most a batch of packets per read.
#[test]
fn hostile_input_never_panics_and_is_bounded_per_packet() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let many: Vec<_> = (0..40)
        .map(|i| dev(i, Pointer::Relative, i % 3 == 0, &"\u{7}".repeat(300)))
        .collect();
    let mut t = T::new(FakeSink {
        devices: many,
        unmapped: vec![1, 2, 3],
        ..FakeSink::default()
    });
    t.take();
    let input_types = [
        EV_KEY, EV_BTN, EV_ABS, EV_REL, EV_WHEEL, EV_GRAB, EV_SURFACE,
    ];
    let mut packets = 0usize;
    for round in 0..200 {
        let n = 1 + (rng.next() % 150) as usize; // often more than one batch
        for _ in 0..n {
            let r = rng.next();
            let ty = if r & 7 == 0 {
                (r >> 8) as u16 // anything, CLOSE and FRAME and unknown types included
            } else {
                input_types[(r >> 8) as usize % input_types.len()]
            };
            t.raw(&Pkt {
                ty,
                flags: (rng.next() & u64::from(F_GRABBED)) as u16,
                seq: rng.next() as u32,
                x: rng.i32(),
                y: rng.i32(),
                w0: rng.i32() as u32,
                w1: rng.i32() as u32,
            });
            packets += 1;
        }
        if round == 199 {
            // a truncated packet: carried, never decoded alone
            let p = Pkt {
                ty: EV_KEY,
                x: 30,
                y: 1,
                ..Pkt::default()
            }
            .encode();
            t.wire.borrow_mut().inbox.extend(&p[..PKT_SIZE - 5]);
        }
        // drain: each read takes at most a batch, and costs a bounded number of calls
        loop {
            let before = t.sink.calls.len();
            let inputs = t.read();
            let calls = t.sink.calls.len() - before;
            assert!(
                calls <= MAX_SINK_CALLS_PER_INPUT * inputs,
                "{calls} sink calls for {inputs} inputs"
            );
            assert!(inputs <= kf_broker::conn::READ_BATCH);
            if t.wire.borrow().inbox.len() < PKT_SIZE {
                break;
            }
        }
        if !t.relay.active() || round % 50 == 49 {
            // anything that ended the connection, and a broker restart now and then
            t.reconnect();
        }
    }
    let calls = t.take();
    let total = t.policy.counters().sink_calls;
    assert!(
        total <= (2 * MAX_SINK_CALLS_PER_INPUT * packets + 2) as u64,
        "{total} sink calls for {packets} packets"
    );
    for c in &calls {
        if let Call::Key(code, _) = c {
            assert!(kf_broker::input::is_evdev_key(*code), "{code:#x}");
        }
        if let Call::Wheel(dx, dy, _) = c {
            assert!(dx.abs() <= 1 && dy.abs() <= 1 && (*dx != 0 || *dy != 0));
        }
        if let Call::Resize(w, h, _) = c {
            assert!((64..=8192).contains(w) && (64..=8192).contains(h));
        }
    }
    assert!(packets > 10_000, "{packets}");
    // not vacuous: the stream reached every verb, and both kinds of refusal fired
    let c = t.policy.counters();
    assert!(
        c.keys > 100
            && c.keys_refused > 10
            && c.buttons > 0
            && c.buttons_dropped > 100
            && c.abs > 100
            && c.rel > 100
            && c.wheel > 100
            && c.switches > 10,
        "{c:?}"
    );
    assert!(t.relay.counters().reconnects >= 4);
}

// ── sink 2: an evdev-style recorder — the same verbs as a virtio-input event stream ─────────

const EV_SYN_T: u16 = 0;
const EV_KEY_T: u16 = 1;
const EV_REL_T: u16 = 2;
const EV_ABS_T: u16 = 3;
const REL_X: u16 = 0;
const REL_Y: u16 = 1;
const REL_HWHEEL: u16 = 6;
const REL_WHEEL: u16 = 8;
const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;

/// What a VMM with two virtio-input pointers and a keyboard (cloud-hypervisor, crosvm,
/// Firecracker) would put on each device's event queue: `(device, type, code, value)`.
#[derive(Default)]
struct EvdevSink {
    events: Vec<(&'static str, u16, u16, i32)>,
    /// The device a queued pointer event went to, flushed by `sync`.
    pending: Vec<&'static str>,
    selected_relative: bool,
}

impl EvdevSink {
    fn dev(to: Pointer) -> &'static str {
        match to {
            Pointer::Absolute => "tablet",
            Pointer::Relative => "mouse",
        }
    }
    fn push(&mut self, d: &'static str, ty: u16, code: u16, v: i32) {
        self.events.push((d, ty, code, v));
        if !self.pending.contains(&d) {
            self.pending.push(d);
        }
    }
}

impl InputSink for EvdevSink {
    fn key(&mut self, code: u16, down: bool) -> bool {
        self.events
            .push(("keyboard", EV_KEY_T, code, i32::from(down)));
        self.events.push(("keyboard", EV_SYN_T, 0, 0));
        true
    }
    fn button(&mut self, b: Button, down: bool, to: Pointer) {
        self.push(Self::dev(to), EV_KEY_T, b.evdev(), i32::from(down));
    }
    fn wheel(&mut self, dx: i32, dy: i32, to: Pointer) {
        if dy != 0 {
            self.push(Self::dev(to), EV_REL_T, REL_WHEEL, dy);
        }
        if dx != 0 {
            self.push(Self::dev(to), EV_REL_T, REL_HWHEEL, dx);
        }
    }
    fn abs(&mut self, x: u32, y: u32, r: AbsRange) {
        let (sx, sy) = r.scale(x, y, 32767);
        self.push("tablet", EV_ABS_T, ABS_X, sx as i32);
        self.push("tablet", EV_ABS_T, ABS_Y, sy as i32);
    }
    fn rel(&mut self, dx: i32, dy: i32) {
        self.push("mouse", EV_REL_T, REL_X, dx);
        self.push("mouse", EV_REL_T, REL_Y, dy);
    }
    fn sync(&mut self) {
        for d in std::mem::take(&mut self.pending) {
            self.events.push((d, EV_SYN_T, 0, 0));
        }
    }
    fn pointer_devices(&mut self, out: &mut [PointerDevice]) -> usize {
        let d = [
            dev(0, Pointer::Absolute, true, "virtio-tablet"),
            dev(1, Pointer::Relative, true, "virtio-mouse"),
        ];
        let n = d.len().min(out.len());
        out[..n].clone_from_slice(&d[..n]);
        n
    }
    fn select_pointer(&mut self, _: u32, kind: Pointer) {
        // two separate virtio devices: nothing to switch, only noted
        self.selected_relative = kind == Pointer::Relative;
    }
    fn missing_pointer(&mut self, _: Pointer) {}
    fn close(&mut self, _: PowerRequest) {}
    fn resize_hint(&mut self, _: u32, _: u32, _: u32) {}
}

/// ★ The trait is not QEMU-shaped: the same broker packets through the same policy drive a sink
/// with no notion of QEMU's handlers, buttons-as-wheel or consoles, and come out as the evdev
/// stream a virtio-input tablet, mouse and keyboard carry.
#[test]
fn an_evdev_style_sink_gets_a_plain_virtio_input_stream() {
    let mut t = T::new(EvdevSink::default());
    t.pkt(EV_KEY, 42, 1, 0, 0);
    t.pkt(EV_KEY, 30, 1, 0, 0);
    t.pkt(EV_ABS, 100, 100, 1920, 1080);
    t.pkt(EV_BTN, 0x110, 1, 0, 0);
    t.pkt(EV_WHEEL, 0, 1, 0, 0);
    t.read();
    t.grabbed(EV_GRAB, 1, 0, 0, 0);
    t.grabbed(EV_REL, 3, -2, 0, 0);
    t.grabbed(EV_REL, 4, 0, 0, 0);
    t.grabbed(EV_WHEEL, -1, 0, 0, 0);
    t.read();
    assert!(t.sink.selected_relative);
    assert_eq!(
        t.sink.events,
        vec![
            ("keyboard", EV_KEY_T, 42, 1),
            ("keyboard", EV_SYN_T, 0, 0),
            ("keyboard", EV_KEY_T, 30, 1),
            ("keyboard", EV_SYN_T, 0, 0),
            ("tablet", EV_ABS_T, ABS_X, 1706),
            ("tablet", EV_ABS_T, ABS_Y, 3033),
            ("tablet", EV_SYN_T, 0, 0),
            ("tablet", EV_KEY_T, 0x110, 1),
            ("tablet", EV_SYN_T, 0, 0),
            ("tablet", EV_REL_T, REL_HWHEEL, 1),
            ("tablet", EV_SYN_T, 0, 0),
            ("mouse", EV_REL_T, REL_X, 7),
            ("mouse", EV_REL_T, REL_Y, -2),
            ("mouse", EV_SYN_T, 0, 0),
            ("mouse", EV_REL_T, REL_WHEEL, -1),
            ("mouse", EV_SYN_T, 0, 0),
        ]
    );
}
