// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★ The relay against nvkvm-pv's REAL, UNCHANGED broker (`--backend test`, which drives no
//! display and takes its input from stdin, `nb_session_test.c`).
//!
//! Ignored by default — it needs the broker binary, built from nvkvm-pv `368d2db`:
//!
//! ```text
//! git -C <nvkvm-pv> archive 368d2db src/broker src/common | tar -x -C <scratch>
//! make -C <scratch>/src/broker nvkvm-display-broker
//! KF_BROKER_BIN=<scratch>/src/broker/nvkvm-display-broker \
//!     cargo test -p kf-broker --test broker_loopback -- --ignored --test-threads=1
//! ```
//!
//! Run with `--ignored` but without `KF_BROKER_BIN`, every test FAILS (it does not pass
//! vacuously). The dma-buf cases need `/dev/udmabuf` and print `UDMABUF-GATE: SKIPPED` without
//! it; the two squatter cases need root and say so when it is not (they need no broker binary).

use kf_broker::wire::{Cmd, EV_FORMAT, FOURCC_XR24, MOD_INVALID, MOD_LINEAR, PKT_SIZE, Pkt};
use kf_broker::{FrameGeom, FrameRing, Host, Input, Relay, RelayConfig, SlotFds, UnixLink};
use kf_linux_raw::{HostPageSize, SharedRam, udmabuf_create};
use std::io::{BufRead as _, BufReader, Write as _};
use std::os::fd::AsFd as _;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const W: u32 = 640;
const H: u32 = 480;

fn broker_bin() -> PathBuf {
    std::env::var_os("KF_BROKER_BIN").map_or_else(
        || {
            panic!(
                "KF_BROKER_BIN is not set: build nvkvm-pv's broker at 368d2db (see this file's \
                 header) — this test does not pass without the real broker"
            )
        },
        PathBuf::from,
    )
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "kfb-loop-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A running broker: its stdin (scripted input) and everything it logged.
struct Broker {
    child: Child,
    stdin: Option<ChildStdin>,
    log: Arc<Mutex<String>>,
}

impl Broker {
    fn spawn(sock: &Path) -> Broker {
        let _ = std::fs::remove_file(sock);
        let mut child = Command::new(broker_bin())
            .args([
                "--backend",
                "test",
                "--size",
                &format!("{W}x{H}"),
                "--socket",
            ])
            .arg(sock)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn the broker");
        let log = Arc::new(Mutex::new(String::new()));
        let err = child.stderr.take().unwrap();
        let l = log.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                eprintln!("[broker] {line}");
                l.lock().unwrap().push_str(&line);
                l.lock().unwrap().push('\n');
            }
        });
        let t0 = Instant::now();
        while !sock.exists() {
            assert!(
                t0.elapsed() < Duration::from_secs(5),
                "the broker never bound"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        Broker {
            stdin: child.stdin.take(),
            child,
            log,
        }
    }

    fn say(&mut self, line: &str) {
        let s = self.stdin.as_mut().unwrap();
        writeln!(s, "{line}").unwrap();
        s.flush().unwrap();
    }

    fn log(&self) -> String {
        self.log.lock().unwrap().clone()
    }

    fn signal(&self, sig: &str) {
        let ok = Command::new("kill")
            .args([sig, &self.child.id().to_string()])
            .status()
            .unwrap()
            .success();
        assert!(ok, "kill {sig}");
    }

    fn kill9(mut self) {
        self.signal("-KILL");
        let _ = self.child.wait();
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Default)]
struct H0 {
    timer: Option<u64>,
}

impl Host for H0 {
    fn watch(&mut self, _fd: i32, _r: bool, _w: bool) {}
    fn timer(&mut self, d: Option<u64>) {
        self.timer = d;
    }
}

/// The relay, its ring and a monotonic clock, driven by polling (level-triggered semantics:
/// a readable/writable call with nothing waiting is an `EAGAIN` and costs nothing).
struct Rig {
    relay: Relay<UnixLink>,
    ring: Arc<FrameRing>,
    host: H0,
    t0: Instant,
    inputs: Vec<Input>,
    serial: u64,
    /// The longest single relay call — the "never blocks" grade.
    worst: Duration,
}

impl Rig {
    fn new(sock: &Path, dmabuf: bool) -> Rig {
        let page = HostPageSize::query();
        let dev = dmabuf.then(|| kf_linux_raw::udmabuf_gate::open_device().expect("udmabuf"));
        let ring = Arc::new(FrameRing::new(kf_broker::slots::BROKER_SLOTS, true));
        for j in 0..ring.slots() {
            let cap = u64::from(W * H * 4);
            let mem = SharedRam::create_named(c"kayfabe-display-frame", cap).unwrap();
            let dma = dev
                .as_ref()
                .map(|d| udmabuf_create(d.as_fd(), &mem, page).expect("UDMABUF_CREATE"));
            ring.install(j, SlotFds::new(mem, dma).unwrap()).unwrap();
        }
        let relay = Relay::new(
            RelayConfig {
                path: sock.to_path_buf(),
                extra_uid: None,
            },
            ring.clone(),
            UnixLink,
        );
        Rig {
            relay,
            ring,
            host: H0::default(),
            t0: Instant::now(),
            inputs: Vec::new(),
            serial: 0,
            worst: Duration::ZERO,
        }
    }

    fn now(&self) -> u64 {
        u64::try_from(self.t0.elapsed().as_millis()).unwrap()
    }

    fn timed(&mut self, f: impl FnOnce(&mut Rig)) {
        let t = Instant::now();
        f(self);
        self.worst = self.worst.max(t.elapsed());
    }

    fn start(&mut self) {
        self.timed(|r| {
            let now = r.now();
            r.relay.start(now, &mut r.host);
        });
    }

    fn publish(&mut self, fourcc: u32) -> usize {
        let t = self.ring.fill_target(None).expect("a target");
        self.serial += 1;
        self.ring.describe(
            t,
            FrameGeom {
                width: W,
                height: H,
                stride: W * 4,
                fourcc,
                serial: self.serial,
            },
        );
        self.ring.publish(t);
        self.timed(|r| {
            let now = r.now();
            r.relay.on_frame(now, &mut r.host);
        });
        t
    }

    /// Pump the relay for `ms`, or until `done` holds.
    fn pump_until(&mut self, ms: u64, mut done: impl FnMut(&Rig) -> bool) -> bool {
        let until = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < until {
            self.timed(|r| {
                let now = r.now();
                let mut out = Vec::new();
                r.relay.on_socket(
                    now,
                    r.relay.connected(),
                    r.relay.connected(),
                    &mut r.host,
                    &mut out,
                    64,
                );
                r.relay.on_timer(now, &mut r.host);
                r.inputs.extend(out);
            });
            if done(self) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        false
    }

    fn pump(&mut self, ms: u64) {
        let _ = self.pump_until(ms, |_| false);
    }
}

fn wait_log(b: &Broker, needle: &str, ms: u64) -> bool {
    let until = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < until {
        if b.log().contains(needle) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

/// ★ Rung 1: a real 640x480 udmabuf, LINEAR — the broker logs OUR dma-buf's id, answers with
/// SURFACE, RELEASE carrying that id, and FRAME; scripted input arrives decoded.
#[test]
#[ignore = "needs KF_BROKER_BIN (nvkvm-pv's broker at 368d2db)"]
fn linear_udmabuf_frames_release_and_scripted_input() {
    let _ = broker_bin();
    kf_linux_raw::require_udmabuf!("linear_udmabuf_frames_release_and_scripted_input");
    let dir = scratch("linear");
    let sock = dir.join("display.sock");
    let mut b = Broker::spawn(&sock);
    let mut r = Rig::new(&sock, true);
    let j = r.publish(FOURCC_XR24);
    let id = r.ring.fds(j).unwrap().dmabuf_id().unwrap();
    r.start();
    assert!(r.pump_until(3000, |r| r.relay.active()), "connected");
    assert!(
        r.pump_until(3000, |r| r.relay.counters().releases >= 1),
        "the replayed frame was RELEASEd by its id"
    );
    let want = format!(
        "TEST attach: id={id} {W}x{H} stride={} offset=0 XR24 mod=0x0000000000000000",
        W * 4
    );
    assert!(
        b.log().contains(&want),
        "broker log lacks `{want}`:\n{}",
        b.log()
    );
    assert_eq!(r.relay.counters().unknown_releases, 0);
    // live frames: each attached, committed, released, paced by FRAME
    for _ in 0..5 {
        r.publish(FOURCC_XR24);
        r.pump(50);
    }
    assert!(r.pump_until(2000, |r| r.relay.counters().releases >= 6));
    assert!(
        r.ring.held_mask().count_ones() <= 1,
        "every frame came back but the retained latest one"
    );
    // input, scripted through the broker's stdin (focus first: unfocused, it sends none)
    for l in [
        "f 1",
        "p 1",
        "k 30 1",
        "k 30 0",
        "b 272 1",
        "a 100 200",
        "w 1 0",
        "r 3 4",
    ] {
        b.say(l);
    }
    let got = r.pump_until(2000, |r| {
        r.inputs.iter().any(|i| matches!(i, Input::Wheel { .. }))
    });
    assert!(got, "inputs so far: {:?}", r.inputs);
    let i = &r.inputs;
    assert!(
        i.contains(&Input::Key {
            code: 30,
            down: true
        }),
        "{i:?}"
    );
    assert!(
        i.contains(&Input::Key {
            code: 30,
            down: false
        }),
        "{i:?}"
    );
    assert!(
        i.contains(&Input::Btn {
            code: 272,
            down: true
        }),
        "{i:?}"
    );
    assert!(
        i.iter()
            .any(|e| matches!(e, Input::Abs { x: 100, y: 200, .. })),
        "{i:?}"
    );
    assert!(i.contains(&Input::Wheel { up: true }), "{i:?}");
    assert!(
        r.worst < Duration::from_millis(50),
        "a relay call took {:?}",
        r.worst
    );
}

/// ★ The broker's answers for the rung-1b pair and for a pair it does not advertise, asked
/// through the same codec (the relay's 1b path itself is in `relay_machine.rs`: the test
/// backend always sets CAP_MODIFIERS, so it never takes 1b).
#[test]
#[ignore = "needs KF_BROKER_BIN (nvkvm-pv's broker at 368d2db)"]
fn the_broker_answers_query_format_for_the_implicit_modifier() {
    use kf_broker::Link as _;
    let dir = scratch("query");
    let sock = dir.join("display.sock");
    let _b = Broker::spawn(&sock);
    let mut link = UnixLink;
    let s = link.connect(&sock).expect("connect");
    let ab24 = u32::from_le_bytes(*b"AB24");
    for (fourcc, m) in [
        (FOURCC_XR24, MOD_INVALID),
        (FOURCC_XR24, MOD_LINEAR),
        (ab24, MOD_LINEAR),
    ] {
        assert_eq!(
            link.send(&s, &Cmd::query(fourcc, m).encode(), None),
            kf_broker::Sent::Done
        );
    }
    let mut got = Vec::new();
    let mut buf = [0u8; PKT_SIZE * 16];
    let mut have = Vec::new();
    let t0 = Instant::now();
    while got.len() < 3 && t0.elapsed() < Duration::from_secs(3) {
        if let kf_broker::Recv::Bytes(n) = link.recv(&s, &mut buf) {
            have.extend_from_slice(&buf[..n]);
        }
        while have.len() >= PKT_SIZE {
            let p = Pkt::decode(have[..PKT_SIZE].try_into().unwrap());
            have.drain(..PKT_SIZE);
            if p.ty == EV_FORMAT {
                got.push((p.x, p.y as u32, p.wide()));
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        got,
        vec![
            (1, FOURCC_XR24, MOD_INVALID),
            (1, FOURCC_XR24, MOD_LINEAR),
            (0, ab24, MOD_LINEAR)
        ]
    );
}

/// ★ Rung 2 (no udmabuf): F_SHM frames, and after `kill -9` and a restart the relay reconnects
/// within its backoff and REPLAYS the last frame WITH F_SHM (nvkvm-pv's replay dropped it).
#[test]
#[ignore = "needs KF_BROKER_BIN (nvkvm-pv's broker at 368d2db)"]
fn shm_frames_and_an_shm_replay_after_kill_9() {
    let dir = scratch("shm");
    let sock = dir.join("display.sock");
    let b = Broker::spawn(&sock);
    let mut r = Rig::new(&sock, false);
    r.start();
    assert!(r.pump_until(3000, |r| r.relay.active()));
    let j = r.publish(FOURCC_XR24);
    let id = r.ring.fds(j).unwrap().memfd_id();
    assert!(r.pump_until(3000, |r| r.relay.counters().releases >= 1));
    assert!(
        b.log().contains(&format!("TEST attach: id={id} {W}x{H}")),
        "{}",
        b.log()
    );
    let last = r.publish(FOURCC_XR24);
    let last_id = r.ring.fds(last).unwrap().memfd_id();
    assert!(r.pump_until(3000, |r| r.relay.counters().releases >= 2));
    b.kill9();
    assert!(
        r.pump_until(3000, |r| !r.relay.connected()),
        "the loss is noticed"
    );
    let b2 = Broker::spawn(&sock);
    assert!(
        r.pump_until(8000, |r| r.relay.active()),
        "reconnected within the backoff"
    );
    assert!(
        wait_log(&b2, &format!("TEST attach: id={last_id} {W}x{H}"), 3000),
        "the new broker was shown the LAST frame, as shared memory:\n{}",
        b2.log()
    );
    assert_eq!(r.relay.counters().reconnects, 1);
}

/// ★ A rejected ATTACH (a fourcc the test backend does not advertise, with no opaque twin) gets
/// no RELEASE; its slot is reclaimed by rule and the path keeps flowing.
#[test]
#[ignore = "needs KF_BROKER_BIN (nvkvm-pv's broker at 368d2db)"]
fn a_rejected_attach_is_reclaimed_by_rule() {
    let dir = scratch("reject");
    let sock = dir.join("display.sock");
    let b = Broker::spawn(&sock);
    let mut r = Rig::new(&sock, false);
    r.start();
    assert!(r.pump_until(3000, |r| r.relay.active()));
    let bad = r.publish(u32::from_le_bytes(*b"AB24"));
    r.pump(200);
    assert!(
        wait_log(&b, "cannot be presented as shared memory", 2000),
        "the broker rejected it:\n{}",
        b.log()
    );
    assert_ne!(
        r.ring.held_mask() & (1 << bad),
        0,
        "no RELEASE comes for it"
    );
    // newer good frames: after 1 s and a newer commit the rejected slot comes back
    for _ in 0..15 {
        r.publish(FOURCC_XR24);
        r.pump(100);
    }
    // (the slot itself may already carry a newer, good frame: it was free to be refilled)
    let c = r.relay.counters();
    assert!(c.reclaims >= 1, "reclaimed by rule: {c:?}");
    assert!(c.releases >= 10, "good frames kept flowing: {c:?}");
    assert!(
        c.blocked <= 12,
        "the wait for the reclaim is bounded: {c:?}"
    );
    assert!(r.ring.held_mask().count_ones() <= 2);
}

/// ★ SIGSTOP the broker for 5 s while frames keep coming: the relay never blocks (every call
/// returns at once), frames wait on the held cap or are reclaimed, and after SIGCONT the path
/// resumes on the same connection.
#[test]
#[ignore = "needs KF_BROKER_BIN (nvkvm-pv's broker at 368d2db)"]
fn a_stopped_broker_never_blocks_the_relay() {
    let dir = scratch("stop");
    let sock = dir.join("display.sock");
    let b = Broker::spawn(&sock);
    let mut r = Rig::new(&sock, false);
    r.start();
    assert!(r.pump_until(3000, |r| r.relay.active()));
    b.signal("-STOP");
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(5) {
        r.publish(FOURCC_XR24);
        r.pump(16);
    }
    let c = r.relay.counters();
    assert!(
        r.worst < Duration::from_millis(50),
        "a relay call took {:?}",
        r.worst
    );
    assert!(c.blocked + c.backstops + c.reclaims > 0, "{c:?}");
    b.signal("-CONT");
    let before = c.releases;
    for _ in 0..10 {
        r.publish(FOURCC_XR24);
        r.pump(50);
    }
    assert!(r.relay.active(), "the same connection survived");
    assert!(
        r.relay.counters().releases > before,
        "RELEASEs resumed after SIGCONT"
    );
}

/// ★ A squatter listening on the path as another uid (65534) is refused BEFORE its HELLO is
/// read, in a world-writable directory root owns. Needs root (no broker binary).
#[test]
#[ignore = "needs root"]
fn a_squatter_under_another_uid_is_refused_before_hello() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = scratch("squat");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
    squatter_is_refused(&dir);
}

/// ★ The review's case (2026-10-03): the squatter OWNS the socket's directory — as anyone does
/// who runs `mkdir /tmp/kf3` first after a reboot. The old rule admitted the directory's owner;
/// it must be refused like any other uid. Needs root (no broker binary).
#[test]
#[ignore = "needs root"]
fn a_squatter_who_owns_the_sockets_directory_is_refused() {
    let dir = scratch("squatdir");
    let ok = Command::new("chown")
        .args(["65534:65534"])
        .arg(&dir)
        .status()
        .expect("chown")
        .success();
    assert!(ok, "chown the directory to the squatter");
    use std::os::unix::fs::MetadataExt as _;
    assert_eq!(std::fs::metadata(&dir).unwrap().uid(), 65534);
    squatter_is_refused(&dir);
}

/// A python3 listener running as uid 65534 binds `dir/display.sock` and would answer HELLO; the
/// relay must refuse it on `SO_PEERCRED` before reading a byte.
fn squatter_is_refused(dir: &Path) {
    assert_eq!(
        kf_broker::effective_uid(),
        0,
        "this case needs root (to run the squatter as uid 65534)"
    );
    use std::os::unix::process::CommandExt as _;
    let sock = dir.join("display.sock");
    let script = format!(
        "import socket,struct,time\ns=socket.socket(socket.AF_UNIX)\ns.bind({:?})\ns.listen(8)\n\
         c,_=s.accept()\ntry:\n  c.send(struct.pack('<HHIiiII',1,0,0,0,0,2,0x3ff))\nexcept OSError:\n  pass\n\
         time.sleep(5)\n",
        sock.to_str().unwrap()
    );
    let mut squat = Command::new("python3")
        .args(["-c", &script])
        .uid(65534)
        .gid(65534)
        .spawn()
        .expect("python3");
    let t0 = Instant::now();
    while !sock.exists() {
        assert!(
            t0.elapsed() < Duration::from_secs(5),
            "the squatter never bound"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut r = Rig::new(&sock, false);
    r.start();
    r.pump(500);
    let _ = squat.kill();
    let _ = squat.wait();
    let c = r.relay.counters();
    assert!(!r.relay.active(), "never connected to a squatter");
    assert!(c.peer_refused >= 1 && c.packets == 0, "{c:?}");
    let _ = std::fs::remove_dir_all(dir);
}
