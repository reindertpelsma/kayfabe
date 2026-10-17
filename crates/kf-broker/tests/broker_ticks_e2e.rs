// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ Phase 2 (docs/design/V3_DISPLAY_PACING_20261017.md §11): the Rust relay against the REAL broker
//! of this repository (`--backend test`, whose display clock is VIRTUAL and driven from stdin) over a
//! real unix socket — the wire halves (opt-in, `EV_TICK`, requests, `COMMIT.seq`, `NO_CHANGE`) agree
//! byte for byte, the lock step holds end to end, silence falls back and ticks resume.
//!
//! Ignored by default (needs the broker binary):
//!
//! ```text
//! make -C broker nvkvm-display-broker
//! KF_BROKER_BIN=$PWD/broker/nvkvm-display-broker \
//!     cargo test --release -p kf-broker --test broker_ticks_e2e -- --ignored --test-threads=1
//! ```

use kf_broker::wire::FOURCC_XR24;
use kf_broker::{FrameGeom, FrameRing, Host, Input, Relay, RelayConfig, SlotFds, TickShare, UnixLink};
use kf_linux_raw::SharedRam;
use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const W: u32 = 64;
const H: u32 = 64;

fn bin() -> PathBuf {
    PathBuf::from(std::env::var_os("KF_BROKER_BIN").expect("KF_BROKER_BIN: the broker binary"))
}

struct Broker {
    child: Child,
    stdin: ChildStdin,
    log: Arc<Mutex<String>>,
}

impl Broker {
    fn spawn(sock: &Path) -> Broker {
        let _ = std::fs::remove_file(sock);
        let mut child = Command::new(bin())
            .args(["--backend", "test", "--size", &format!("{W}x{H}"), "--socket"])
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
                let mut g = l.lock().unwrap();
                g.push_str(&line);
                g.push('\n');
            }
        });
        let t0 = Instant::now();
        while !sock.exists() {
            assert!(t0.elapsed() < Duration::from_secs(5), "the broker never bound");
            std::thread::sleep(Duration::from_millis(10));
        }
        let stdin = child.stdin.take().unwrap();
        Broker { child, stdin, log }
    }

    fn say(&mut self, line: &str) {
        writeln!(self.stdin, "{line}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn log(&self) -> String {
        self.log.lock().unwrap().clone()
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct H0;
impl Host for H0 {
    fn watch(&mut self, _fd: i32, _r: bool, _w: bool) {}
    fn timer(&mut self, _d: Option<u64>) {}
}

struct Rig {
    relay: Relay<UnixLink>,
    ring: Arc<FrameRing>,
    share: Arc<TickShare>,
    t0: Instant,
    serial: u64,
}

impl Rig {
    fn new(sock: &Path) -> Rig {
        let ring = Arc::new(FrameRing::new(kf_broker::slots::BROKER_SLOTS, true));
        for j in 0..ring.slots() {
            let mem = SharedRam::create_named(c"kayfabe-display-frame", u64::from(W * H * 4)).unwrap();
            ring.install(j, SlotFds::new(mem, None).unwrap()).unwrap();
        }
        let share = Arc::new(TickShare::new());
        share.set_cap_hz(60);
        let relay = Relay::new(
            RelayConfig {
                path: sock.to_path_buf(),
                extra_uid: None,
            },
            ring.clone(),
            UnixLink,
        )
        .with_ticks(share.clone());
        Rig {
            relay,
            ring,
            share,
            t0: Instant::now(),
            serial: 0,
        }
    }

    fn now(&self) -> u64 {
        u64::try_from(self.t0.elapsed().as_millis()).unwrap()
    }

    fn pump(&mut self, ms: u64) {
        let until = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < until {
            let now = self.now();
            let mut out: Vec<Input> = Vec::new();
            let c = self.relay.connected();
            self.relay.on_socket(now, c, c, &mut H0, &mut out, 64);
            self.relay.on_timer(now, &mut H0);
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn publish(&mut self) {
        let t = self.ring.fill_target(None).expect("a target");
        self.serial += 1;
        self.ring.describe(
            t,
            FrameGeom {
                width: W,
                height: H,
                stride: W * 4,
                fourcc: FOURCC_XR24,
                serial: self.serial,
                cursor: false,
            },
        );
        self.ring.publish(t);
        let now = self.now();
        self.relay.on_frame(now, &mut H0);
    }
}

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("kfb-tick-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// ★ The whole phase-2 exchange against the real broker.
#[test]
#[ignore = "needs KF_BROKER_BIN (this repository's broker)"]
fn the_relay_and_the_real_broker_agree_on_ticks_requests_and_silence() {
    let sock = dir("e2e").join("display.sock");
    let mut b = Broker::spawn(&sock);
    let mut r = Rig::new(&sock);
    r.relay.start(0, &mut H0);
    r.pump(1500);
    assert!(r.relay.active(), "connected: {}", b.log());
    assert!(r.relay.ticks_negotiated(), "the relay opted in");
    assert!(b.log().contains("ticks: ON"), "the broker saw the opt-in:\n{}", b.log());
    // the priming tick carries a request (WANT_FRAME) and arrives at once
    assert!(r.share.snapshot(0).is_some(), "the priming tick");
    // 60 ticks at 60 Hz on the broker's virtual clock, paced so each leaves before the next
    let mut ns = 5_000_000_000u64;
    let mut frames_wanted = 0;
    for i in 0..60 {
        ns += 16_666_667;
        b.say(&format!("t {ns} 16666667"));
        r.pump(8);
        if i == 10 || i == 30 {
            r.publish();
            frames_wanted += 1;
        }
        r.pump(6);
        b.say("P");
    }
    r.pump(50);
    let c = r.relay.tick_counters();
    assert!(c.ticks >= 55, "ticks reached the relay: {c:?}");
    assert_eq!((c.ticks_stale, c.ts_backwards, c.partial_timeouts), (0, 0, 0), "{c:?}");
    assert!(c.want_ticks >= 20, "the broker opens requests: {c:?}");
    assert!(
        c.frames_on_credit >= frames_wanted,
        "each published frame went on a request: {c:?}\n{}",
        b.log()
    );
    assert!(c.no_change_sent >= 5, "idle requests are answered NO_CHANGE: {c:?}");
    // the broker accounted every answer and saw no violation, no late or unsolicited reply
    b.say("S");
    r.pump(20);
    let log = b.log();
    let stats = log.lines().rev().find(|l| l.contains("TEST tick stats")).expect("stats").to_string();
    assert!(stats.contains("late=0 unsolicited=0"), "{stats}");
    assert!(!log.contains("VIOLATION") && !log.contains("violation"), "{log}");
    assert!(r.relay.active());
    assert!(r.share.snapshot(0).unwrap().count >= 55);
    // silence: no ticks for 80 ms of REAL time -> the relay says silent, the clock takes over
    ns += 16_666_667;
    b.say(&format!("t {ns} 16666667"));
    r.pump(15);
    assert!(r.relay.tick_live(), "live while ticks flow");
    let fallbacks = r.relay.tick_counters().silence_fallbacks;
    r.pump(80);
    assert!(!r.relay.tick_live(), "50 ms of silence: not live");
    assert_eq!(r.relay.tick_counters().silence_fallbacks, fallbacks + 1);
    // and the next tick resumes
    ns += 16_666_667;
    b.say(&format!("t {ns} 16666667"));
    r.pump(30);
    assert!(r.relay.tick_live(), "the next tick resumes");
    assert_eq!(r.relay.tick_counters().silence_fallbacks, fallbacks + 1, "no flap on resume");
    eprintln!("{}", r.relay.tick_status());
}
