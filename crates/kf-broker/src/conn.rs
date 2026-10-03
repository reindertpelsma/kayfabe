// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★★ **The relay's connection machine** — the VMM side of nvkvm-pv's broker protocol v2.
//!
//! Protocol semantics are ported from nvkvm-pv `src/qemu/nvkvm_display_relay.c` at `368d2db`
//! (Apache-2.0, same author; under kayfabe's grant, `LICENSE`). The port is a REWRITE of its
//! state machine with every invariant of `docs/design/V3_DISPLAY.md` §8.1 kept and tested, and
//! these deliberate differences:
//!
//! - **No CONNECTING state.** An `AF_UNIX` stream `connect` succeeds or fails at once; `EAGAIN`
//!   is a full backlog, i.e. a failed attempt that goes to the backoff (nvkvm-pv treated it as
//!   "in progress", `relay.c:1573-1576`).
//! - **The peer is checked right after `connect`, before a byte is read** (`SO_PEERCRED`
//!   against uid 0, this process's effective uid READ AT THAT ATTEMPT, and
//!   [`RelayConfig::extra_uid`]). Whoever listens on the path gets the guest's screen and types
//!   into the guest; nvkvm-pv never checked (`relay.c:1553-1600`). ⊘ The owner of the socket's
//!   directory is not trusted (corrected 2026-10-03): anyone can create a missing directory
//!   under `/tmp`.
//! - **The first attempt is made on the timer, never inside [`Relay::start`]**: QEMU drops
//!   privileges (`-run-with user=`, `-runas`) after realize and before its main loop runs, so
//!   the euid the peer check reads is the one QEMU runs as.
//! - **A later "no" wins**: a broker that probes an import after advertising it sends an
//!   unsolicited `EV_FORMAT x=0` (`nb_session_wl.c:610-626`); nvkvm-pv kept the first answer
//!   (`relay.c:1371-1373`). Here a 0 downgrades the rung and reclaims every held frame attached
//!   under that pair.
//! - **Pacing**: a COMMIT spends the one credit; `EV_FRAME`, or the `RELEASE` of the most
//!   recently committed buffer (the X11 XRender path never sends FRAME, nvkvm-pv audit S-7),
//!   returns it; a 100 ms backstop covers a hidden window.
//! - **The ATTACH flags travel with the frame** on the live, owed AND replay paths (nvkvm-pv's
//!   replay dropped `F_SHM`, `relay.c:1965-1973`).
//! - **Rung 1b** — the implicit modifier (`MOD_INVALID`) for a broker without `CAP_MODIFIERS`.
//! - ★ **Rung 0, the GPU copy** (`V3_DISPLAY.md` §8.11, `OWNER_RULINGS.md` §L, 2026-10-03): a
//!   frame packed into a VRAM object kayfabe owns goes as a block-linear dma-buf
//!   (`DRM_FORMAT_MOD_NVIDIA_BLOCK_LINEAR_2D`), only on an EXPLICIT yes to
//!   `QUERY_FORMAT(XR24, that modifier)` with `CAP_MODIFIERS` and `CAP_RELEASE`, never while the
//!   broker says its compositor renders on another GPU (`EV_DEVICE`), and never while the
//!   acknowledgement detector backs off: X11 reports a refused DRI3 import NOWHERE, so a native
//!   frame unconfirmed by a RELEASE after [`DETECT_COMMITS`] commits and [`DETECT_MS`] backs the
//!   rung off for [`BACKOFF_MIN_MS`] (doubling to [`BACKOFF_MAX_MS`]) — a timed back-off, never a
//!   permanent "no", because on X11 with the NVIDIA DDX the host rungs may well be a black window.
//!   The relay's per-connection choice reaches the display worker through one atomic
//!   ([`FrameRing::want_vram`]).
//! - All per-connection knowledge lives in one [`Conn`] that is dropped on disconnect, so the
//!   "reconnect inherits partial state" class (nvkvm-pv audits B-2, S-11, RR-07) cannot be
//!   written.
//!
//! It is VMM-agnostic: socket I/O goes through [`Link`], fd-handler and timer registration
//! through [`Host`], and input comes back as [`Input`] for the VMM to inject. Every entry takes
//! the time (`now_ms`), so the machine is deterministic under test. Every syscall is
//! non-blocking; nothing here waits.

use crate::slots::{FrameRing, Kind, MAX_SLOTS, Take};
use crate::wire::{
    CAP_DMABUF, CAP_FOCUS_EVENTS, CAP_MODIFIERS, CAP_RELEASE, CLOSE_FORCE, CMD_ATTACH, CMD_F_SHM,
    CMD_SIZE, Cmd, EV_ABS, EV_BTN, EV_BYE, EV_CLIPBOARD, EV_CLOSE, EV_DEVICE, EV_FOCUS, EV_FORMAT,
    EV_FRAME, EV_GRAB, EV_HELLO, EV_KEY, EV_POINTER, EV_REL, EV_RELEASE, EV_SURFACE, EV_WHEEL,
    FOURCC_XR24, MOD_INVALID, MOD_LINEAR, PKT_SIZE, PROTO_VERSION, Pkt, fourcc_name,
};
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// First reconnect delay; doubled per failed attempt up to [`RETRY_MAX_MS`] (`relay.c:504-505`).
pub const RETRY_MIN_MS: u64 = 200;
/// The longest reconnect delay.
pub const RETRY_MAX_MS: u64 = 5000;
/// connect + peer check + HELLO + replay must finish within this (`relay.c:1597-1598`).
pub const HANDSHAKE_MS: u64 = 2000;
/// How long an owed frame waits for writability before it is retried anyway (`relay.c:132`).
pub const OWED_MS: u64 = 50;
/// The pacing backstop: a credit spent this long ago returns without FRAME or RELEASE.
pub const CREDIT_BACKSTOP_MS: u64 = 100;
/// A held frame with no RELEASE becomes reclaimable this long after its commit (and only once a
/// newer frame was committed).
pub const RECLAIM_AFTER_MS: u64 = 1000;
/// The most packets one read takes (`kf3_broker_ready`'s batch).
pub const READ_BATCH: usize = 64;
/// Distinct (fourcc, modifier) verdicts remembered per connection (`relay.c:92`).
const VERDICTS: usize = 4;
/// ★ The GPU-copy acknowledgement detector: this many native commits with no RELEASE naming a
/// native frame …
pub const DETECT_COMMITS: u32 = 3;
/// … and at least this long since the first of them, trip a back-off.
pub const DETECT_MS: u64 = 1000;
/// The first back-off from the GPU-copy rung; doubled per trip …
pub const BACKOFF_MIN_MS: u64 = 5000;
/// … up to this; a native RELEASE resets it.
pub const BACKOFF_MAX_MS: u64 = 60_000;
/// The largest Linux input code (`KEY_MAX`); anything above is not an evdev code.
const KEY_MAX: i32 = 0x2ff;

/// What one record send did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sent {
    /// The whole record went.
    Done,
    /// The socket is full (`EAGAIN`): nothing went.
    Full,
    /// Anything else, a short write included — fatal for the connection.
    Failed(String),
}

/// What one read did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recv {
    /// Bytes read.
    Bytes(usize),
    /// Nothing waiting (`EAGAIN`).
    Empty,
    /// End of stream.
    Closed,
    /// The peer attached a descriptor, which the kernel dropped (`MSG_CTRUNC`): a violation.
    FdDropped,
    /// Anything else.
    Failed(String),
}

/// ★ The socket operations the machine needs. [`crate::UnixLink`] is the real one.
pub trait Link {
    /// A connected socket; dropping it closes it.
    type Sock;
    /// Connect to `path` (non-blocking; every error is a failed attempt).
    ///
    /// # Errors
    /// Why not, for the log.
    fn connect(&mut self, path: &Path) -> Result<Self::Sock, String>;
    /// The descriptor number to watch.
    fn fd(&self, sock: &Self::Sock) -> i32;
    /// The peer's uid (`SO_PEERCRED`).
    ///
    /// # Errors
    /// Why it could not be read.
    fn peer_uid(&mut self, sock: &Self::Sock) -> Result<u32, String>;
    /// ★ This process's effective uid NOW — asked at every attempt, never cached (a VMM that
    /// dropped privileges after realize is judged as what it runs as).
    fn effective_uid(&mut self) -> u32;
    /// Send one command record with at most one descriptor.
    fn send(&mut self, sock: &Self::Sock, rec: &[u8; CMD_SIZE], fd: Option<BorrowedFd<'_>>)
    -> Sent;
    /// Read into `buf`, accepting no descriptor.
    fn recv(&mut self, sock: &Self::Sock, buf: &mut [u8]) -> Recv;
}

/// ★ What the VMM registers for the machine — called only from inside the machine's entries.
pub trait Host {
    /// Watch `fd` for readability/writability; `(false, false)` removes the handler. Always
    /// called with `(false, false)` BEFORE the descriptor is closed.
    fn watch(&mut self, fd: i32, read: bool, write: bool);
    /// Fire the machine's timer at `deadline_ms` (the same clock as `now_ms`), or never.
    fn timer(&mut self, deadline_ms: Option<u64>);
}

/// ★ Input for the VMM to inject, every value already bounded (the broker is the trusted side
/// of the socket, but trusted is not unbounded, `relay.c:1872-1873`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    /// A Linux evdev key code (`0..=KEY_MAX`); the VMM still checks it against its own map.
    Key {
        /// The code.
        code: u16,
        /// Pressed.
        down: bool,
    },
    /// A Linux evdev button code (`BTN_LEFT` …).
    Btn {
        /// The code.
        code: u16,
        /// Pressed.
        down: bool,
    },
    /// An absolute position in a `w` x `h` range, clamped into it; `w`, `h` > 0.
    Abs {
        /// Column.
        x: i32,
        /// Row.
        y: i32,
        /// Range width.
        w: i32,
        /// Range height.
        h: i32,
    },
    /// Relative motion (consecutive packets summed, saturating).
    Rel {
        /// Columns.
        dx: i32,
        /// Rows.
        dy: i32,
    },
    /// One vertical wheel detent (horizontal detents are not mapped by QEMU 10.2's virtio or
    /// USB pointers, so they are dropped).
    Wheel {
        /// Up (away from the user).
        up: bool,
    },
    /// The broker grabbed (or released) the pointer: switch to a relative (absolute) device.
    Grab(bool),
    /// The user closed the window: `force` = stop the machine now, else an ACPI powerdown.
    Close {
        /// The user chose "force off".
        force: bool,
    },
    /// ★ Display step 3c: the broker's window is now `w` x `h` (clamped to 64..=8192) at `mhz`
    /// millihertz (0: unknown) — the VMM hands it to its UI layer as a resize hint (QEMU
    /// coalesces for 1 s); whether a windowed resize re-modes the guest is the broker's
    /// `--resolution` policy, not the relay's.
    Surface {
        /// Width.
        w: i32,
        /// Height.
        h: i32,
        /// Refresh in millihertz, 0 when the broker does not know.
        mhz: u32,
    },
}

/// The relay's configuration.
#[derive(Debug, Clone)]
pub struct RelayConfig {
    /// The broker's socket (absolute; checked at realize).
    pub path: PathBuf,
    /// The `display-broker-uid` property: one more uid accepted as the broker, besides root and
    /// the VMM's effective uid at each attempt ([`crate::broker_uids`]).
    pub extra_uid: Option<u32>,
}

/// A presentation rung (`V3_DISPLAY.md` §8.2, §8.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rung {
    /// 0: the GPU copy — a block-linear dma-buf of the slot's kayfabe-owned VRAM backing.
    Native,
    /// 1: the udmabuf as a LINEAR dma-buf.
    Linear,
    /// 1b: the udmabuf with the implicit modifier, for a broker without `CAP_MODIFIERS`.
    Implicit,
    /// 2: the memfd as shared memory (`F_SHM`).
    Shm,
}

/// Counters, for the log and the device's status line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    /// Connections that reached ACTIVE.
    pub connected: u64,
    /// Of those, after a previous one.
    pub reconnects: u64,
    /// Attempts that failed (connect, peer, HELLO, replay, deadline).
    pub attempts_failed: u64,
    /// Of those, refused by the peer check.
    pub peer_refused: u64,
    /// Frames the worker announced.
    pub frames_announced: u64,
    /// Frames committed (live, owed and replay).
    pub sent: u64,
    /// ATTACHes the socket would not take.
    pub dropped: u64,
    /// COMMITs the socket would not take after their ATTACH went.
    pub uncommitted: u64,
    /// Owed frames later delivered.
    pub recovered: u64,
    /// RELEASEs matched to a held frame.
    pub releases: u64,
    /// RELEASEs naming nothing held.
    pub unknown_releases: u64,
    /// Held frames reclaimed without a RELEASE.
    pub reclaims: u64,
    /// Times a ready frame waited because two were held.
    pub blocked: u64,
    /// Credits returned by the backstop.
    pub backstops: u64,
    /// Broker packets processed.
    pub packets: u64,
    /// Frames refused before the wire (no backing, an impossible geometry).
    pub refused: u64,
    /// ★ Of `sent`, frames committed on the GPU-copy rung.
    pub native: u64,
    /// GPU-copy back-offs the acknowledgement detector tripped.
    pub native_trips: u64,
}

/// What the broker said about its compositor's device (`EV_DEVICE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Device {
    /// Nothing said (an older broker), or "cannot tell".
    Unknown,
    /// One of this GPU's DRM nodes.
    Same,
    /// Another device: the GPU-copy rung is not offered on this connection.
    Other(u32, u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Asked,
    Yes,
    No,
}

#[derive(Debug, Clone, Copy)]
struct VerdictRow {
    fourcc: u32,
    modifier: u64,
    v: Verdict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Window,
    Attach,
    Commit,
    Caps,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Hello,
    Replay(Step),
    Active,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owed {
    /// The broker never heard of this frame: WINDOW (if stale), ATTACH, COMMIT are owed.
    Attach(usize),
    /// The ATTACH went: only the COMMIT is owed (re-sending the ATTACH would import twice).
    Commit(usize),
}

/// ★ A frame the broker may be reading: what was sent for it, and when.
#[derive(Debug, Clone, Copy)]
struct Held {
    /// The id of the descriptor actually sent (its rung's), or `None` before the ATTACH went.
    sent_id: Option<u64>,
    rung: Rung,
    fourcc: u32,
    modifier: u64,
    /// When it was attached (or committed, once it was).
    at_ms: u64,
    /// The commit counter at that moment.
    commit_no: u64,
    /// The broker released it while it was the latest commit: it is kept only as the frame a
    /// reconnect replays, and goes back when a newer frame is committed.
    released: bool,
    /// ★ Its ATTACH went but its owed COMMIT was SUPERSEDED by a newer frame: no COMMIT will
    /// ever follow that ATTACH (the next one follows the newer frame's ATTACH), so the broker
    /// never shows it and sends no RELEASE for it. It yields its slot to a live frame, like a
    /// retained one — without this, it and the latest commit filled the cap of 2 and nothing
    /// could ever be committed again (the review's freeze, 2026-10-03).
    superseded: bool,
}

/// ★ Everything known about ONE broker connection — dropped whole on disconnect.
struct Conn<S> {
    sock: S,
    fd: i32,
    phase: Phase,
    caps: u32,
    rx: [u8; PKT_SIZE],
    rxlen: usize,
    window: Option<(u32, u32)>,
    verdicts: Vec<VerdictRow>,
    owed: Option<Owed>,
    owed_at: Option<u64>,
    credit: bool,
    credit_at: Option<u64>,
    last_commit: Option<usize>,
    handshake_until: u64,
    replay: Option<usize>,
    /// A send returned `Full` and the machine waits for writability (replay or owed).
    want_write: bool,
    /// What the VMM currently watches the socket for.
    watched: (bool, bool),
    rung_logged: Option<Rung>,
    /// The broker window's last reported size (SURFACE is logged on a change only: some backends
    /// send it with every frame).
    surface: Option<(i32, i32)>,
    seq: u32,
    /// ★ The compositor's device (`EV_DEVICE`).
    device: Device,
    /// ★ The acknowledgement detector: a RELEASE named a native frame on this connection.
    native_confirmed: bool,
    /// Native commits since the last confirmation, and when the first of them went.
    native_unacked: u32,
    native_unacked_since: Option<u64>,
    /// No native frame before this; the next back-off's length.
    backoff_until: Option<u64>,
    backoff_ms: u64,
}

/// ★ The relay. VM-lifetime: it outlives connections, holds the backoff, the held-frame table
/// (which mirrors the ring's held mask) and the counters.
pub struct Relay<L: Link> {
    link: L,
    cfg: RelayConfig,
    ring: Arc<FrameRing>,
    conn: Option<Conn<L::Sock>>,
    retry_at: Option<u64>,
    retry_ms: u64,
    retry_logged: bool,
    ever_connected: bool,
    held: [Option<Held>; MAX_SLOTS],
    commits: u64,
    timer: Option<u64>,
    stopped: bool,
    powerdown: Option<(u64, u32)>,
    counters: Counters,
    /// The time of the entry being run (every public entry sets it first).
    now_ms: u64,
}

macro_rules! say {
    ($($t:tt)*) => { $crate::conn::log_line(format!($($t)*)) };
}

thread_local! {
    static CAPTURE: core::cell::RefCell<Option<Vec<String>>> =
        const { core::cell::RefCell::new(None) };
}

/// Every relay log line goes through here: to stderr, prefixed `kf3: broker:` — and, while a
/// [`capture_log`] guard lives on this thread, into its capture too.
pub fn log_line(line: String) {
    eprintln!("kf3: broker: {line}");
    CAPTURE.with(|c| {
        if let Some(v) = c.borrow_mut().as_mut() {
            v.push(line);
        }
    });
}

/// Test support: what the relay logged on this thread while the guard lived.
#[doc(hidden)]
#[derive(Debug)]
pub struct LogCapture(());

impl LogCapture {
    /// The lines logged so far.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        CAPTURE.with(|c| c.borrow().clone().unwrap_or_default())
    }
}

impl Drop for LogCapture {
    fn drop(&mut self) {
        CAPTURE.with(|c| *c.borrow_mut() = None);
    }
}

/// Test support: start capturing this thread's relay log lines (a relay is driven from one
/// thread — QEMU's main loop — so a test's capture sees exactly its own relay's lines).
#[doc(hidden)]
#[must_use]
pub fn capture_log() -> LogCapture {
    CAPTURE.with(|c| *c.borrow_mut() = Some(Vec::new()));
    LogCapture(())
}

/// A repeating line is said for the first four and then every 256th time — never a line per
/// frame (30-60 a second) for as long as the condition lasts.
fn loud(n: u64) -> bool {
    n <= 4 || n.is_multiple_of(256)
}

impl<L: Link> Relay<L> {
    /// A relay over `ring` (which must feed the broker); nothing happens until [`Relay::start`].
    #[must_use]
    pub fn new(cfg: RelayConfig, ring: Arc<FrameRing>, link: L) -> Relay<L> {
        Relay {
            link,
            cfg,
            ring,
            conn: None,
            retry_at: None,
            retry_ms: RETRY_MIN_MS,
            retry_logged: false,
            ever_connected: false,
            held: [None; MAX_SLOTS],
            commits: 0,
            timer: None,
            stopped: false,
            powerdown: None,
            counters: Counters::default(),
            now_ms: 0,
        }
    }

    /// The counters.
    #[must_use]
    pub fn counters(&self) -> Counters {
        self.counters
    }

    /// Whether a connection is ACTIVE.
    #[must_use]
    pub fn active(&self) -> bool {
        self.conn.as_ref().is_some_and(|c| c.phase == Phase::Active)
    }

    /// Whether any connection (handshaking or active) exists.
    #[must_use]
    pub fn connected(&self) -> bool {
        self.conn.is_some()
    }

    /// The broker's capabilities on the current connection (0 without one).
    #[must_use]
    pub fn caps(&self) -> u32 {
        self.conn.as_ref().map_or(0, |c| c.caps)
    }

    /// The socket's descriptor number, while connected.
    #[must_use]
    pub fn socket_fd(&self) -> Option<i32> {
        self.conn.as_ref().map(|c| c.fd)
    }

    /// One status fragment.
    #[must_use]
    pub fn status(&self) -> String {
        let c = &self.counters;
        format!(
            "broker[up={} sent={} gpucopy={} gpucopy_backoffs={} dropped={} uncommitted={} recovered={} releases={} unknown_releases={} reclaims={} blocked={} backstops={} reconnects={} failed_attempts={} peer_refused={}]",
            u8::from(self.active()),
            c.sent,
            c.native,
            c.native_trips,
            c.dropped,
            c.uncommitted,
            c.recovered,
            c.releases,
            c.unknown_releases,
            c.reclaims,
            c.blocked,
            c.backstops,
            c.reconnects,
            c.attempts_failed,
            c.peer_refused
        )
    }

    /// ★ Arm the first attempt for `now` on the machine's timer — it is made from the VMM's
    /// loop, never inside this call. ⊘ QEMU realizes devices BEFORE it drops privileges
    /// (`os_setup_post`: `-run-with user=`, `-runas`), and only then runs its main loop; an
    /// attempt made here would judge the peer against root's euid and refuse a broker running
    /// as the uid QEMU is about to become. ⊘ Not a startup dependency: a failure is logged once
    /// and retried in the background; the VM boots regardless.
    pub fn start(&mut self, now: u64, host: &mut dyn Host) {
        self.now_ms = now;
        if self.stopped || self.conn.is_some() || self.retry_at.is_some() {
            return;
        }
        self.retry_at = Some(now);
        self.timer = Some(now);
        host.timer(Some(now));
    }

    /// ★ The socket is readable and/or writable. Reads at most [`READ_BATCH`] packets (`cap`
    /// lower) plus a carried partial one; level-triggered readiness delivers the rest. Input
    /// for the VMM is appended to `out` (at most `cap` entries).
    pub fn on_socket(
        &mut self,
        now: u64,
        readable: bool,
        writable: bool,
        host: &mut dyn Host,
        out: &mut Vec<Input>,
        cap: usize,
    ) {
        self.now_ms = now;
        if writable {
            self.on_writable(now, host);
        }
        if readable && self.conn.is_some() {
            self.read_batch(now, host, out, cap.clamp(1, READ_BATCH));
        }
        self.finish(now, host);
    }

    /// ★ The worker published a frame.
    pub fn on_frame(&mut self, now: u64, host: &mut dyn Host) {
        self.now_ms = now;
        self.counters.frames_announced += 1;
        self.progress(now, host);
        self.finish(now, host);
    }

    /// ★ The machine's timer fired (or any wakeup: deadlines are checked on every entry).
    pub fn on_timer(&mut self, now: u64, host: &mut dyn Host) {
        self.now_ms = now;
        self.finish(now, host);
    }

    /// ★ Stop for good: unwatch, close, no timer. Held frames are released.
    pub fn stop(&mut self, host: &mut dyn Host) {
        self.stopped = true;
        self.close(host);
        self.retry_at = None;
        self.timer = None;
        host.timer(None);
    }

    // ── connection lifecycle ──────────────────────────────────────────────────────────────

    fn attempt(&mut self, now: u64, host: &mut dyn Host) {
        self.retry_at = None;
        let sock = match self.link.connect(&self.cfg.path) {
            Ok(s) => s,
            Err(e) => {
                let why = format!(
                    "cannot connect to {}: {e} — is the display broker running?",
                    self.cfg.path.display()
                );
                self.failed(now, host, &why);
                return;
            }
        };
        // ★ the peer check, before a single byte is read — against the euid of THIS moment
        let euid = self.link.effective_uid();
        let allowed = crate::link::broker_uids(euid, self.cfg.extra_uid);
        match self.link.peer_uid(&sock) {
            Ok(uid) if allowed.contains(&uid) => {}
            Ok(uid) => {
                drop(sock); // never watched, never read
                self.counters.peer_refused += 1;
                let why = format!(
                    "REFUSED the listener on {}: it runs as uid {uid}, which is not an allowed \
                     broker (allowed: uid 0, QEMU's effective uid {euid}{}). If uid {uid} is \
                     your broker, set display-broker-uid={uid} on the device or run QEMU as that \
                     user (-run-with user=)",
                    self.cfg.path.display(),
                    self.cfg
                        .extra_uid
                        .map_or_else(String::new, |x| format!(", display-broker-uid {x}"))
                );
                self.failed(now, host, &why);
                return;
            }
            Err(e) => {
                drop(sock);
                self.failed(now, host, &format!("SO_PEERCRED: {e}"));
                return;
            }
        }
        let fd = self.link.fd(&sock);
        self.conn = Some(Conn {
            sock,
            fd,
            phase: Phase::Hello,
            caps: 0,
            rx: [0; PKT_SIZE],
            rxlen: 0,
            window: None,
            verdicts: Vec::new(),
            owed: None,
            owed_at: None,
            credit: true,
            credit_at: None,
            last_commit: None,
            handshake_until: now + HANDSHAKE_MS,
            replay: None,
            want_write: false,
            watched: (true, false),
            rung_logged: None,
            surface: None,
            seq: 0,
            device: Device::Unknown,
            native_confirmed: false,
            native_unacked: 0,
            native_unacked_since: None,
            backoff_until: None,
            backoff_ms: BACKOFF_MIN_MS,
        });
        host.watch(fd, true, false);
    }

    /// Unwatch, then close; give back every held frame except the latest commit, which becomes
    /// the next connection's replay frame when nothing newer is ready.
    fn close(&mut self, host: &mut dyn Host) {
        let Some(c) = self.conn.take() else { return };
        // no connection, no GPU-copy demand: the worker stops packing at its next frame
        self.ring.set_want_vram(false);
        host.watch(c.fd, false, false);
        drop(c.sock);
        if let Some(j) = c.last_commit
            && self.held[j].is_some()
        {
            let _ = self.ring.requeue(j);
        }
        self.ring.clear_held();
        self.held = [None; MAX_SLOTS];
    }

    /// A handshake-stage failure: loud once, then silent; backoff doubles.
    fn failed(&mut self, now: u64, host: &mut dyn Host, why: &str) {
        self.counters.attempts_failed += 1;
        self.close(host);
        if !self.retry_logged {
            self.retry_logged = true;
            say!("connection attempt failed: {why}");
            say!(
                "retrying in the background (up to every {RETRY_MAX_MS} ms); the VM continues running"
            );
        }
        self.schedule_retry(now);
    }

    /// An ACTIVE connection was lost: say what it carried, retry from the shortest delay.
    fn dropped(&mut self, now: u64, host: &mut dyn Host, why: &str) {
        self.close(host);
        let c = &self.counters;
        say!(
            "{why}; the display and input are gone for now ({} frames relayed, {} dropped, {} \
             attached without a commit, {} releases, {} reclaimed). The VM keeps running; \
             reconnecting in the background.",
            c.sent,
            c.dropped,
            c.uncommitted,
            c.releases,
            c.reclaims
        );
        self.retry_ms = RETRY_MIN_MS;
        self.retry_logged = false;
        self.schedule_retry(now);
    }

    fn schedule_retry(&mut self, now: u64) {
        if self.stopped {
            return;
        }
        self.retry_at = Some(now + self.retry_ms);
        self.retry_ms = (self.retry_ms * 2).min(RETRY_MAX_MS);
    }

    fn lost(&mut self, now: u64, host: &mut dyn Host, why: &str) {
        if self.active() {
            self.dropped(now, host, why);
        } else {
            self.failed(now, host, why);
        }
    }

    // ── sending ──────────────────────────────────────────────────────────────────────────

    fn send(&mut self, cmd: &Cmd, slot_fd: Option<usize>, rung: Rung) -> Sent {
        let Some(c) = self.conn.as_ref() else {
            return Sent::Failed("no connection".into());
        };
        let rec = cmd.encode();
        let fd = slot_fd.and_then(|j| match rung {
            Rung::Native => self.ring.vram(j).map(crate::slots::VramFds::fd),
            Rung::Shm => self.ring.fds(j).map(crate::SlotFds::memfd),
            Rung::Linear | Rung::Implicit => self.ring.fds(j).and_then(crate::SlotFds::dmabuf),
        });
        if slot_fd.is_some() && fd.is_none() {
            return Sent::Failed("an ATTACH without its descriptor".into());
        }
        self.link.send(&c.sock, &rec, fd)
    }

    /// ★ Whether the GPU-copy rung may be used on this connection NOW, and its modifier: the
    /// device offers it (a modifier from the render node), the broker negotiates modifiers and
    /// sends real RELEASEs, it said an EXPLICIT yes to `(XR24, modifier)`, its compositor is not
    /// on another GPU, and the detector is not backing off.
    fn native_allowed(&self) -> Option<u64> {
        let m = self.ring.vram_modifier()?;
        let c = self.conn.as_ref()?;
        let ok = c.phase != Phase::Hello
            && c.caps & CAP_MODIFIERS != 0
            && c.caps & CAP_RELEASE != 0
            && verdict(c, FOURCC_XR24, m) == Some(Verdict::Yes)
            && !matches!(c.device, Device::Other(..))
            && c.backoff_until.is_none_or(|t| self.now_ms >= t);
        ok.then_some(m)
    }

    /// ★ The rung for `slot`'s published frame on this connection and its modifier — `None` when
    /// no backing that holds the frame can be sent (none fresh, its kind withdrawn, or a geometry
    /// that does not fit it). Rung 0 first, then the host rungs as before (§8.2).
    fn choose(&self, slot: usize) -> Option<(Rung, u64)> {
        let g = self.ring.geometry(slot);
        if g.width == 0
            || g.height == 0
            || g.width > crate::wire::MAX_DIM
            || g.height > crate::wire::MAX_DIM
        {
            return None;
        }
        let w4 = u64::from(g.width) * 4;
        if g.fourcc == FOURCC_XR24
            && self.ring.backed(slot, Kind::Vram)
            && let Some(m) = self.native_allowed()
            && let Some(v) = self.ring.vram(slot)
        {
            let vg = self.ring.vram_geometry(slot);
            let stride = u64::from(vg.stride);
            // the broker's own bounds (`4w <= stride <= 8w + 4096`, `stride * h <= size`), and
            // the extent inside the object the GPU wrote
            if stride >= w4
                && stride <= 2 * w4 + 4096
                && stride * u64::from(g.height) <= vg.extent
                && vg.extent <= v.bytes()
            {
                return Some((Rung::Native, m));
            }
        }
        if !self.ring.backed(slot, Kind::Host) {
            return None;
        }
        let fds = self.ring.fds(slot)?;
        if u64::from(g.stride) < w4 || u64::from(g.stride) * u64::from(g.height) > fds.bytes() {
            return None;
        }
        let Some(c) = self.conn.as_ref() else {
            return Some((Rung::Shm, MOD_LINEAR));
        };
        if fds.dmabuf().is_some() {
            if c.caps & CAP_MODIFIERS != 0 {
                // an unknown verdict counts as yes (`relay.c:753-793`)
                if verdict(c, g.fourcc, MOD_LINEAR) != Some(Verdict::No) {
                    return Some((Rung::Linear, MOD_LINEAR));
                }
            } else if verdict(c, g.fourcc, MOD_INVALID) == Some(Verdict::Yes) {
                return Some((Rung::Implicit, MOD_INVALID));
            }
        }
        Some((Rung::Shm, MOD_LINEAR))
    }

    /// Ask once per connection whether the dma-buf rung for `fourcc` can be shown — and, when
    /// this device offers the GPU-copy rung, whether the block-linear pair can
    /// ([`Self::ensure_native_query`]).
    fn ensure_query(&mut self, fourcc: u32, slot: usize) {
        self.ensure_native_query();
        let has_dmabuf = self.ring.fds(slot).is_some_and(|f| f.dmabuf().is_some());
        let Some(c) = self.conn.as_ref() else { return };
        if !has_dmabuf {
            return;
        }
        let modifier = if c.caps & CAP_MODIFIERS != 0 {
            MOD_LINEAR
        } else {
            MOD_INVALID
        };
        self.ask(fourcc, modifier);
    }

    /// ★ Ask `(XR24, the GPU-copy modifier)` once per connection, when the device offers the rung
    /// and the broker negotiates modifiers — whether or not a slot has a VRAM backing yet: with
    /// `display-broker-vram=auto` the slots are provisioned only at the first explicit yes.
    fn ensure_native_query(&mut self) {
        let Some(m) = self.ring.vram_modifier() else {
            return;
        };
        if self
            .conn
            .as_ref()
            .is_some_and(|c| c.caps & CAP_MODIFIERS != 0)
        {
            self.ask(FOURCC_XR24, m);
        }
    }

    /// `QUERY_FORMAT(fourcc, modifier)` unless this connection already asked.
    fn ask(&mut self, fourcc: u32, modifier: u64) {
        let Some(c) = self.conn.as_ref() else { return };
        if verdict(c, fourcc, modifier).is_some() {
            return;
        }
        if self.send(&Cmd::query(fourcc, modifier), None, Rung::Shm) == Sent::Done
            && let Some(c) = self.conn.as_mut()
        {
            if c.verdicts.len() == VERDICTS {
                c.verdicts.remove(0); // full: recycle the oldest
            }
            c.verdicts.push(VerdictRow {
                fourcc,
                modifier,
                v: Verdict::Asked,
            });
        }
        // a refused query is not fatal and not retried here: the next frame asks again
    }

    /// A frame just taken from the ring, not yet sent.
    fn claimed(&self, slot: usize, now: u64) -> Held {
        Held {
            sent_id: None,
            rung: Rung::Shm,
            fourcc: self.ring.geometry(slot).fourcc,
            modifier: 0,
            at_ms: now,
            commit_no: self.commits,
            released: false,
            superseded: false,
        }
    }

    /// Whether `slot` may be sent: some rung can carry its published frame now ([`Self::choose`]
    /// — a fresh backing of a kind not withdrawn, holding its geometry inside the broker's
    /// bounds). ★ The per-kind withdrawal check is what stops a held frame requeued for a replay
    /// after [`FrameRing::withdraw`]: the ring requeues whatever was held.
    fn fits(&self, slot: usize) -> bool {
        self.choose(slot).is_some()
    }

    /// A frame just claimed that [`Self::fits`] refused: counted, logged at a bounded rate
    /// ([`loud`]) with the reason, and its slot given back — on the live path and the replay's
    /// alike (⊘ the replay's refusal used to be silent; since 2026-10-03 a held frame requeued
    /// after [`FrameRing::withdraw_all`] is refused there, and says so).
    fn refuse_unfit(&mut self, j: usize) {
        self.counters.refused += 1;
        let n = self.counters.refused;
        if loud(n) {
            if self.ring.broker_backed(j) {
                let g = self.ring.geometry(j);
                say!(
                    "REFUSED frame slot {j}: no current backing fits its {}x{} geometry ({n} so far)",
                    g.width,
                    g.height
                );
            } else {
                say!(
                    "REFUSED frame slot {j}: no backing holding this frame may be sent — its kind is \
                     withdrawn (a broker frame backing of that kind was refused), or the slot has no \
                     broker backing ({n} so far)"
                );
            }
        }
        self.unhold(j);
    }

    fn attach_cmd(&mut self, slot: usize) -> Option<(Cmd, Rung)> {
        let (rung, modifier) = self.choose(slot)?;
        let g = self.ring.geometry(slot);
        let stride = if rung == Rung::Native {
            self.ring.vram_geometry(slot).stride
        } else {
            g.stride
        };
        let c = self.conn.as_mut()?;
        if c.rung_logged != Some(rung) {
            c.rung_logged = Some(rung);
            match rung {
                Rung::Native => say!(
                    "frames go as a GPU copy in host VRAM (a block-linear dma-buf, modifier \
                     {modifier:#018x}; no byte crosses PCIe, nothing of the guest's is exported)"
                ),
                Rung::Linear => {
                    say!("frames go as a LINEAR dma-buf (udmabuf, zero-copy for the broker)")
                }
                Rung::Implicit => say!(
                    "frames go as a dma-buf with the IMPLICIT modifier (the broker negotiates no modifiers)"
                ),
                Rung::Shm => say!(
                    "frames go as shared memory (F_SHM) — an X11 broker accepts it only with \
                     --present-mode=shm, and a refusal is not reported back"
                ),
            }
        }
        let seq = c.seq;
        c.seq = c.seq.wrapping_add(1);
        Some((
            Cmd {
                ty: CMD_ATTACH,
                flags: if rung == Rung::Shm { CMD_F_SHM } else { 0 },
                width: g.width,
                height: g.height,
                stride,
                offset: 0,
                fourcc: g.fourcc,
                modifier,
                seq,
            },
            rung,
        ))
    }

    fn sent_id(&self, slot: usize, rung: Rung) -> Option<u64> {
        match rung {
            Rung::Native => self.ring.vram(slot).map(crate::slots::VramFds::id),
            Rung::Shm => self.ring.fds(slot).map(crate::SlotFds::memfd_id),
            Rung::Linear | Rung::Implicit => {
                self.ring.fds(slot).and_then(crate::SlotFds::dmabuf_id)
            }
        }
    }

    /// WINDOW if the frame's size is not the last one sent on this connection.
    fn window_if_stale(&mut self, slot: usize) -> Sent {
        let g = self.ring.geometry(slot);
        if self
            .conn
            .as_ref()
            .is_some_and(|c| c.window == Some((g.width, g.height)))
        {
            return Sent::Done;
        }
        let r = self.send(&Cmd::window(g.width, g.height), None, Rung::Shm);
        if r == Sent::Done
            && let Some(c) = self.conn.as_mut()
        {
            if c.phase == Phase::Active {
                say!("guest resolution is now {}x{}", g.width, g.height);
            }
            c.window = Some((g.width, g.height));
        }
        r
    }

    /// ATTACH `slot` (and record what was sent for it). `None`: the slot may no longer be sent
    /// ([`Self::fits`] — the ring was withdrawn by the worker, on its own thread, since the
    /// caller last looked); the caller refuses the frame ([`Self::refuse_unfit`]) and the
    /// connection stays up.
    ///
    /// ⊘ CORRECTED 2026-10-03 (the third review of `v3-broker`): an unfit slot used to come back
    /// as `Sent::Failed`, and every caller treats `Failed` as a dead socket — so an owed ATTACH
    /// (or a replay's) after [`FrameRing::withdraw_all`] tore down a healthy broker connection,
    /// blamed the socket, and lost input until the reconnect.
    fn attach(&mut self, now: u64, slot: usize) -> Option<Sent> {
        let (cmd, rung) = self.attach_cmd(slot)?;
        let r = self.send(&cmd, Some(slot), rung);
        if r == Sent::Done {
            self.held[slot] = Some(Held {
                sent_id: self.sent_id(slot, rung),
                rung,
                fourcc: cmd.fourcc,
                modifier: cmd.modifier,
                at_ms: now,
                commit_no: self.commits,
                released: false,
                superseded: false,
            });
        }
        Some(r)
    }

    /// COMMIT `slot` and spend the credit. `None`: the slot may no longer be sent (withdrawn
    /// since its ATTACH went) — nothing is committed, and the caller refuses the frame. ★ So
    /// the relay commits NO frame once it has observed the withdrawal, the owed COMMIT of a
    /// frame attached before it included: that frame is never shown, like a superseded one.
    fn commit(&mut self, now: u64, slot: usize) -> Option<Sent> {
        if !self.ring.broker_backed(slot) {
            return None;
        }
        let r = self.send(&Cmd::commit(), None, Rung::Shm);
        if r == Sent::Done {
            self.commits += 1;
            self.counters.sent += 1;
            if self.held[slot].is_some_and(|h| h.rung == Rung::Native) {
                self.counters.native += 1;
                if let Some(c) = self.conn.as_mut()
                    && !c.native_confirmed
                {
                    c.native_unacked += 1;
                    c.native_unacked_since.get_or_insert(now);
                }
            }
            if let Some(h) = self.held[slot].as_mut() {
                h.at_ms = now;
                h.commit_no = self.commits;
            }
            let prev = self.conn.as_mut().and_then(|c| {
                c.credit = false;
                c.credit_at = Some(now + CREDIT_BACKSTOP_MS);
                c.last_commit.replace(slot)
            });
            // the previous latest commit, already released, is no longer retained
            if let Some(p) = prev.filter(|p| *p != slot)
                && self.held[p].is_some_and(|h| h.released)
            {
                self.unhold(p);
            }
        }
        Some(r)
    }

    /// Give back a held frame the broker never heard of (or no longer reads).
    fn unhold(&mut self, slot: usize) {
        self.ring.release_held(slot);
        self.held[slot] = None;
    }

    /// ★ The live path: take the broker-ready frame (if credit allows) and send it.
    fn progress(&mut self, now: u64, host: &mut dyn Host) {
        let Some(phase) = self.conn.as_ref().map(|c| c.phase) else {
            return;
        };
        match phase {
            Phase::Hello => return,
            Phase::Replay(Step::Window | Step::Attach) => {
                // a newer frame replaces the replay frame the broker has not heard of yet
                if self.ring.broker_ready().is_some() {
                    if let Some(old) = self.conn.as_ref().and_then(|c| c.replay) {
                        self.unhold(old);
                    }
                    if let Take::Taken(j) = self.ring.take_broker()
                        && let Some(c) = self.conn.as_mut()
                    {
                        c.replay = Some(j);
                        c.phase = Phase::Replay(Step::Window);
                        self.held[j] = Some(self.claimed(j, now));
                    }
                }
                return;
            }
            Phase::Replay(Step::Commit) => {
                // invariant 6: the ATTACH of the replay frame is on the stream; committing it
                // would show a stale frame while claiming the newest was retained
                if self.ring.broker_ready().is_some() {
                    self.failed(
                        now,
                        host,
                        "a newer frame replaced one attached during connection replay",
                    );
                }
                return;
            }
            Phase::Replay(Step::Caps) => return,
            Phase::Active => {}
        }
        // a newer frame supersedes an owed one
        if let Some(owed) = self.conn.as_ref().and_then(|c| c.owed) {
            if self.ring.broker_ready().is_none() {
                return;
            }
            match owed {
                Owed::Attach(j) => self.unhold(j), // never reached the broker
                // its ATTACH is on the stream with no COMMIT to follow: never shown, never
                // released — it must yield its slot (see `Held::superseded`)
                Owed::Commit(j) => {
                    if let Some(h) = self.held[j].as_mut() {
                        h.superseded = true;
                    }
                }
            }
            if let Some(c) = self.conn.as_mut() {
                c.owed = None;
                c.owed_at = None;
                c.want_write = false;
            }
        }
        if !self.conn.as_ref().is_some_and(|c| c.credit) {
            return;
        }
        let j = match self.ring.take_broker() {
            Take::Empty => return,
            Take::Taken(j) => j,
            Take::Full => {
                // a superseded frame (never shown) yields first, then the retained (already
                // released) latest one: neither is on screen, and retention is only for a
                // replay — neither may hold back a live frame
                let yields = (0..MAX_SLOTS)
                    .find(|&j| self.held[j].is_some_and(|h| h.superseded))
                    .or_else(|| (0..MAX_SLOTS).find(|&j| self.held[j].is_some_and(|h| h.released)));
                if let Some(j) = yields {
                    self.unhold(j);
                } else if !self.reclaim_overdue(now) {
                    self.counters.blocked += 1;
                    return;
                }
                match self.ring.take_broker() {
                    Take::Taken(j) => j,
                    _ => return,
                }
            }
        };
        self.held[j] = Some(self.claimed(j, now));
        if !self.fits(j) {
            self.refuse_unfit(j);
            return;
        }
        let fourcc = self.ring.geometry(j).fourcc;
        self.ensure_query(fourcc, j);
        self.send_frame(now, host, j);
    }

    /// WINDOW (if stale) → ATTACH → COMMIT for held `j`, recording what is owed on `Full`. A
    /// slot that stops fitting on the way (withdrawn by the worker meanwhile) is refused, never
    /// a socket failure.
    fn send_frame(&mut self, now: u64, host: &mut dyn Host, j: usize) {
        if !self.fits(j) {
            return self.refuse_unfit(j);
        }
        match self.window_if_stale(j) {
            Sent::Done => {}
            Sent::Full => return self.owe(now, Owed::Attach(j), true),
            Sent::Failed(e) => {
                return self.lost(now, host, &format!("the broker socket failed: {e}"));
            }
        }
        match self.attach(now, j) {
            None => return self.refuse_unfit(j),
            Some(Sent::Done) => {}
            Some(Sent::Full) => return self.owe(now, Owed::Attach(j), true),
            Some(Sent::Failed(e)) => {
                return self.lost(now, host, &format!("the broker socket failed: {e}"));
            }
        }
        match self.commit(now, j) {
            None => self.refuse_unfit(j),
            Some(Sent::Done) => {}
            Some(Sent::Full) => {
                self.counters.uncommitted += 1;
                let n = self.counters.uncommitted;
                if loud(n) {
                    say!(
                        "the broker did not drain: a frame was attached but not committed, so the \
                         display holds the previous one ({n} so far)"
                    );
                }
                self.owe(now, Owed::Commit(j), false);
            }
            Some(Sent::Failed(e)) => self.lost(
                now,
                host,
                &format!("the broker socket failed on commit: {e}"),
            ),
        }
    }

    fn owe(&mut self, now: u64, what: Owed, dropped: bool) {
        if dropped {
            self.counters.dropped += 1;
        }
        if let Some(c) = self.conn.as_mut() {
            c.owed = Some(what);
            c.owed_at = Some(now + OWED_MS);
            c.want_write = true;
        }
    }

    /// ★ Re-send as much of the owed frame as the socket now takes (`relay.c:636-707`). An owed
    /// frame whose slot no longer fits — the worker withdrew the ring since it was owed — is
    /// refused and forgotten ([`Self::drop_owed_unfit`]); the connection stays up.
    fn flush_owed(&mut self, now: u64, host: &mut dyn Host) {
        let Some(owed) = self.conn.as_ref().and_then(|c| c.owed) else {
            return;
        };
        let j = match owed {
            Owed::Attach(j) => {
                if !self.fits(j) {
                    return self.drop_owed_unfit(j);
                }
                // a stale WINDOW first: the owed frame may be the last one, with no next frame
                // to fix the window size
                match self.window_if_stale(j) {
                    Sent::Done => {}
                    Sent::Full => return self.rearm_owed(now),
                    Sent::Failed(e) => {
                        return self.lost(now, host, &format!("resending geometry failed: {e}"));
                    }
                }
                match self.attach(now, j) {
                    None => return self.drop_owed_unfit(j),
                    Some(Sent::Done) => {}
                    Some(Sent::Full) => return self.rearm_owed(now),
                    Some(Sent::Failed(e)) => {
                        return self.lost(now, host, &format!("redelivering a frame failed: {e}"));
                    }
                }
                if let Some(c) = self.conn.as_mut() {
                    c.owed = Some(Owed::Commit(j));
                }
                j
            }
            Owed::Commit(j) => j,
        };
        match self.commit(now, j) {
            None => self.drop_owed_unfit(j),
            Some(Sent::Done) => {
                self.counters.recovered += 1;
                self.clear_owed();
            }
            Some(Sent::Full) => self.rearm_owed(now),
            Some(Sent::Failed(e)) => {
                self.lost(now, host, &format!("redelivering a commit failed: {e}"));
            }
        }
    }

    fn clear_owed(&mut self) {
        if let Some(c) = self.conn.as_mut() {
            c.owed = None;
            c.owed_at = None;
            c.want_write = false;
        }
    }

    /// The owed frame in `j` may no longer be sent: nothing more of it goes (an ATTACH already on
    /// the stream gets no COMMIT, so the broker never shows it), it is refused by name, and its
    /// slot comes back.
    fn drop_owed_unfit(&mut self, j: usize) {
        self.clear_owed();
        self.refuse_unfit(j);
    }

    fn rearm_owed(&mut self, now: u64) {
        if let Some(c) = self.conn.as_mut() {
            c.owed_at = Some(now + OWED_MS);
            c.want_write = true;
        }
    }

    /// ★ The narrowed reclaim rule (`V3_DISPLAY.md` §8.3): a held frame other than the latest
    /// commit, committed (or attached) at least [`RECLAIM_AFTER_MS`] ago, with a newer commit
    /// since. RELEASE is advisory and a rejected ATTACH never gets one, so without this two
    /// rejected frames would freeze the broker path. Returns whether any slot came back.
    fn reclaim_overdue(&mut self, now: u64) -> bool {
        let last = self.conn.as_ref().and_then(|c| c.last_commit);
        let mut any = false;
        for j in 0..MAX_SLOTS {
            let Some(h) = self.held[j] else { continue };
            if Some(j) == last
                || self.conn.as_ref().and_then(|c| c.owed) == Some(Owed::Commit(j))
                || now < h.at_ms + RECLAIM_AFTER_MS
                || self.commits <= h.commit_no
            {
                continue;
            }
            self.unhold(j);
            self.counters.reclaims += 1;
            any = true;
            say!(
                "reclaimed frame slot {j}: no RELEASE {} ms after its commit, and a newer frame \
                 was committed ({} reclaimed so far)",
                now - h.at_ms,
                self.counters.reclaims
            );
        }
        any
    }

    /// The earliest moment [`Self::reclaim_overdue`] can reclaim something. Reclaim is eager —
    /// a frame the broker refused without a word (a rejected ATTACH) would otherwise shrink the
    /// ring for the rest of the connection.
    fn reclaim_deadline(&self) -> Option<u64> {
        let last = self.conn.as_ref().and_then(|c| c.last_commit);
        let owed = self.conn.as_ref().and_then(|c| c.owed);
        self.held
            .iter()
            .enumerate()
            .filter_map(|(j, h)| {
                h.filter(|h| {
                    Some(j) != last && owed != Some(Owed::Commit(j)) && self.commits > h.commit_no
                })
            })
            .map(|h| h.at_ms + RECLAIM_AFTER_MS)
            .min()
    }

    // ── replay ───────────────────────────────────────────────────────────────────────────

    /// HELLO accepted: ask the formats in play, then WINDOW → ATTACH → COMMIT of the newest frame
    /// (with its flags) → CAPS (`relay.c:1935-2008`).
    fn begin_replay(&mut self, now: u64, host: &mut dyn Host) {
        let j = match self.ring.take_broker() {
            Take::Taken(j) => Some(j),
            Take::Empty | Take::Full => None,
        };
        if let Some(j) = j {
            self.held[j] = Some(self.claimed(j, now));
            if !self.fits(j) {
                self.refuse_unfit(j);
            } else {
                let fourcc = self.ring.geometry(j).fourcc;
                self.ensure_query(fourcc, j);
            }
        }
        let j = j.filter(|j| self.held[*j].is_some());
        if let Some(c) = self.conn.as_mut() {
            c.replay = j;
            c.phase = Phase::Replay(Step::Window);
        }
        self.continue_replay(now, host);
    }

    fn continue_replay(&mut self, now: u64, host: &mut dyn Host) {
        loop {
            let Some((Phase::Replay(step), replay)) =
                self.conn.as_ref().map(|c| (c.phase, c.replay))
            else {
                return;
            };
            // ★ a replay frame that stopped fitting (the ring was withdrawn meanwhile) is refused
            // and the replay goes on to CAPS — never a failed attempt
            let unfit = |r: &Relay<L>, j: usize| match step {
                Step::Window | Step::Attach => !r.fits(j),
                Step::Commit => !r.ring.broker_backed(j),
                Step::Caps => false,
            };
            if let Some(j) = replay
                && unfit(self, j)
            {
                self.refuse_unfit(j);
                if let Some(c) = self.conn.as_mut() {
                    c.replay = None;
                }
                continue;
            }
            let (r, next) = match (step, replay) {
                (Step::Window, Some(j)) => (self.window_if_stale(j), Step::Attach),
                (Step::Attach, Some(j)) => match self.attach(now, j) {
                    Some(r) => (r, Step::Commit),
                    None => {
                        self.refuse_unfit(j);
                        if let Some(c) = self.conn.as_mut() {
                            c.replay = None;
                        }
                        continue;
                    }
                },
                (Step::Commit, Some(j)) => {
                    let Some(r) = self.commit(now, j) else {
                        self.refuse_unfit(j);
                        if let Some(c) = self.conn.as_mut() {
                            c.replay = None;
                        }
                        continue;
                    };
                    if r == Sent::Done {
                        let g = self.ring.geometry(j);
                        say!(
                            "re-sent geometry {}x{} and the last frame to the new broker",
                            g.width,
                            g.height
                        );
                    }
                    (r, Step::Caps)
                }
                (Step::Window | Step::Attach | Step::Commit, None) => (Sent::Done, Step::Caps),
                // CAPS last: no clipboard (bit 0 clear)
                (Step::Caps, _) => {
                    let r = self.send(&Cmd::caps(0), None, Rung::Shm);
                    if r == Sent::Done {
                        self.activate(now, host);
                        return;
                    }
                    (r, Step::Caps)
                }
            };
            match r {
                Sent::Done => {
                    if let Some(c) = self.conn.as_mut() {
                        c.phase = Phase::Replay(next);
                        c.want_write = false;
                    }
                }
                Sent::Full => {
                    if let Some(c) = self.conn.as_mut() {
                        c.want_write = true;
                    }
                    return;
                }
                Sent::Failed(e) => {
                    self.failed(
                        now,
                        host,
                        &format!("socket failure during connection replay: {e}"),
                    );
                    return;
                }
            }
        }
    }

    fn activate(&mut self, now: u64, host: &mut dyn Host) {
        let Some(c) = self.conn.as_mut() else { return };
        c.phase = Phase::Active;
        c.replay = None;
        c.want_write = false;
        self.counters.connected += 1;
        if self.ever_connected {
            self.counters.reconnects += 1;
            say!(
                "reconnected to the display broker (reconnect #{})",
                self.counters.reconnects
            );
        }
        self.ever_connected = true;
        self.retry_ms = RETRY_MIN_MS;
        self.retry_logged = false;
        self.progress(now, host);
    }

    // ── receiving ────────────────────────────────────────────────────────────────────────

    fn read_batch(&mut self, now: u64, host: &mut dyn Host, out: &mut Vec<Input>, cap: usize) {
        let budget = cap * PKT_SIZE;
        let mut buf = [0u8; READ_BATCH * PKT_SIZE];
        let mut filled = 0usize;
        let mut end: Option<Recv> = None;
        while filled < budget {
            let Some(c) = self.conn.as_ref() else { return };
            match self.link.recv(&c.sock, &mut buf[filled..budget]) {
                Recv::Bytes(0) | Recv::Closed => {
                    end = Some(Recv::Closed);
                    break;
                }
                Recv::Bytes(n) => filled += n.min(budget - filled),
                Recv::Empty => break,
                r @ (Recv::FdDropped | Recv::Failed(_)) => {
                    end = Some(r);
                    break;
                }
            }
        }
        // the carried partial packet, then whole packets; a new partial is carried again
        let mut at = 0usize;
        while at < filled {
            let Some(c) = self.conn.as_mut() else { return };
            let n = (PKT_SIZE - c.rxlen).min(filled - at);
            c.rx[c.rxlen..c.rxlen + n].copy_from_slice(&buf[at..at + n]);
            c.rxlen += n;
            at += n;
            if c.rxlen < PKT_SIZE {
                break;
            }
            c.rxlen = 0;
            let p = Pkt::decode(&c.rx);
            self.counters.packets += 1;
            self.handle(now, host, &p, out, cap);
        }
        match end {
            Some(Recv::Closed) => {
                let why = if self.active() {
                    "the display broker closed the connection"
                } else {
                    "the display broker closed during the handshake"
                };
                self.lost(now, host, why);
            }
            Some(Recv::FdDropped) => self.lost(
                now,
                host,
                "PROTOCOL VIOLATION: the broker attached a descriptor to an event",
            ),
            Some(Recv::Failed(e)) => {
                self.lost(now, host, &format!("read error on the broker socket: {e}"))
            }
            _ => {}
        }
    }

    fn handle(&mut self, now: u64, host: &mut dyn Host, p: &Pkt, out: &mut Vec<Input>, cap: usize) {
        let Some(phase) = self.conn.as_ref().map(|c| c.phase) else {
            return;
        };
        if phase == Phase::Hello {
            return self.hello(now, host, p);
        }
        let emit = |out: &mut Vec<Input>, i: Input| {
            if out.len() < cap {
                out.push(i);
            }
        };
        match p.ty {
            EV_KEY if (0..=KEY_MAX).contains(&p.x) => emit(
                out,
                Input::Key {
                    code: p.x as u16,
                    down: p.y != 0,
                },
            ),
            EV_BTN if (0..=KEY_MAX).contains(&p.x) => emit(
                out,
                Input::Btn {
                    code: p.x as u16,
                    down: p.y != 0,
                },
            ),
            EV_ABS if p.w0 > 0 && p.w1 > 0 => {
                let w = i32::try_from(p.w0.min(1 << 20)).unwrap_or(1);
                let h = i32::try_from(p.w1.min(1 << 20)).unwrap_or(1);
                emit(
                    out,
                    Input::Abs {
                        x: p.x.clamp(0, w - 1),
                        y: p.y.clamp(0, h - 1),
                        w,
                        h,
                    },
                );
            }
            EV_REL => {
                if let Some(Input::Rel { dx, dy }) = out.last_mut() {
                    *dx = dx.saturating_add(p.x);
                    *dy = dy.saturating_add(p.y);
                } else {
                    emit(out, Input::Rel { dx: p.x, dy: p.y });
                }
            }
            EV_WHEEL if p.x != 0 => emit(out, Input::Wheel { up: p.x > 0 }),
            EV_GRAB => {
                say!("grab {}", if p.x != 0 { "ON" } else { "off" });
                emit(out, Input::Grab(p.x != 0));
            }
            EV_FOCUS => say!(
                "window {}",
                if p.x != 0 {
                    "active"
                } else {
                    "inactive (input suspended)"
                }
            ),
            EV_SURFACE => {
                let (w, h) = (p.x.clamp(64, 8192), p.y.clamp(64, 8192));
                if p.x > 0
                    && p.y > 0
                    && let Some(c) = self.conn.as_mut()
                    && c.surface != Some((w, h))
                {
                    c.surface = Some((w, h));
                    say!("broker window is now {w}x{h}");
                    emit(out, Input::Surface { w, h, mhz: p.w0 });
                }
            }
            EV_FRAME => {
                if let Some(c) = self.conn.as_mut() {
                    c.credit = true;
                    c.credit_at = None;
                }
                self.progress(now, host);
            }
            EV_RELEASE => self.release(now, host, p.wide()),
            EV_FORMAT => self.format(now, host, p),
            EV_CLOSE => {
                let force = p.x == CLOSE_FORCE;
                self.close_policy(now, force);
                emit(out, Input::Close { force });
            }
            EV_BYE => say!("the broker is going away (reason {})", p.x),
            EV_DEVICE => self.device(p),
            // POINTER, a second HELLO, CLIPBOARD (not offered: CAPS bit 0 is clear), the
            // out-of-range values filtered above, and types from a NEWER broker: fixed-size
            // packets, so skipping one is exact
            EV_POINTER | EV_HELLO | EV_CLIPBOARD => {}
            _ => {}
        }
    }

    fn hello(&mut self, now: u64, host: &mut dyn Host, p: &Pkt) {
        let why = if p.ty != EV_HELLO {
            Some("the first broker packet was not HELLO")
        } else if p.w0 != PROTO_VERSION {
            Some("the broker reported an incompatible protocol version")
        } else if p.w1 & CAP_DMABUF == 0 {
            Some("the broker cannot accept dma-buf buffers")
        } else {
            None
        };
        if let Some(why) = why {
            return self.failed(now, host, why);
        }
        let udmabuf =
            (0..self.ring.slots()).any(|j| self.ring.fds(j).is_some_and(|f| f.dmabuf().is_some()));
        say!(
            "connected to {}, capabilities {:#x}{}",
            self.cfg.path.display(),
            p.w1,
            if udmabuf {
                ""
            } else {
                " — no /dev/udmabuf here, so frames go as shared memory only"
            }
        );
        if p.w1 & CAP_FOCUS_EVENTS == 0 {
            say!(
                "NOTE: the broker cannot observe focus loss on this session, so it will not \
                 offer a keyboard grab. Absolute pointer and keyboard still work while the \
                 window is active."
            );
        }
        if let Some(c) = self.conn.as_mut() {
            c.caps = p.w1;
        }
        // ★ ask the block-linear pair at once: with `display-broker-vram=auto` the VRAM slots are
        // provisioned only at the first explicit yes, so it must not wait for a frame
        self.ensure_native_query();
        self.begin_replay(now, host);
    }

    fn release(&mut self, now: u64, host: &mut dyn Host, id: u64) {
        let hit = (0..MAX_SLOTS).find(|&j| self.held[j].is_some_and(|h| h.sent_id == Some(id)));
        let Some(j) = hit else {
            self.counters.unknown_releases += 1;
            return;
        };
        self.counters.releases += 1;
        // ★ a RELEASE names only a buffer the compositor imported: the GPU copy is acknowledged
        if self.held[j].is_some_and(|h| h.rung == Rung::Native)
            && let Some(c) = self.conn.as_mut()
        {
            if !c.native_confirmed {
                say!("the display imported a GPU-copy frame (its RELEASE came back)");
            }
            c.native_confirmed = true;
            c.native_unacked = 0;
            c.native_unacked_since = None;
            c.backoff_until = None;
            c.backoff_ms = BACKOFF_MIN_MS;
        }
        if let Some(c) = self.conn.as_mut()
            && c.last_commit == Some(j)
        {
            // the RELEASE of the latest commit returns the credit (X11 XRender sends no FRAME),
            // and the frame stays RETAINED until a newer one is committed: it is what a
            // reconnecting broker is shown (`relay.c:262-271`'s retained frame)
            c.credit = true;
            c.credit_at = None;
            if let Some(h) = self.held[j].as_mut() {
                h.released = true;
            }
        } else {
            self.unhold(j);
        }
        self.progress(now, host);
    }

    fn format(&mut self, now: u64, host: &mut dyn Host, p: &Pkt) {
        let fourcc = u32::from_le_bytes(p.y.to_le_bytes());
        let modifier = p.wide();
        let Some(c) = self.conn.as_mut() else { return };
        let last = c.last_commit;
        let Some(row) = c
            .verdicts
            .iter_mut()
            .find(|r| r.fourcc == fourcc && r.modifier == modifier)
        else {
            say!(
                "stale EV_FORMAT for {} modifier {modifier:#x}, ignored",
                fourcc_name(fourcc)
            );
            return;
        };
        let was = row.v;
        if p.x == 0 {
            row.v = Verdict::No; // ★ a later "no" always wins
        } else if row.v == Verdict::Asked {
            row.v = Verdict::Yes;
        }
        let v = row.v;
        if v != was {
            say!(
                "the display {} show {} modifier {modifier:#018x}",
                if v == Verdict::Yes { "CAN" } else { "CANNOT" },
                fourcc_name(fourcc)
            );
        }
        if v != Verdict::No {
            return;
        }
        // ★ fix (a): free what that pair orphaned — the broker failed to import them
        let mut credit = false;
        for j in 0..MAX_SLOTS {
            if self.held[j].is_some_and(|h| {
                h.sent_id.is_some()
                    && h.rung != Rung::Shm
                    && h.fourcc == fourcc
                    && h.modifier == modifier
            }) {
                self.unhold(j);
                self.counters.reclaims += 1;
                credit |= Some(j) == last;
                say!("reclaimed frame slot {j}: the display refused its format");
            }
        }
        if credit && let Some(c) = self.conn.as_mut() {
            c.credit = true;
            c.credit_at = None;
        }
        self.progress(now, host);
    }

    /// ★ `EV_DEVICE`: which DRM device the compositor renders on. Another device than this GPU's
    /// nodes ⇒ no GPU-copy frame on this connection (`DupMemory` refuses vidmem from another
    /// device, `ogkm-580: nvkms-kapi.c:1789-1806`; a non-NVIDIA importer gets no `sg_table`).
    fn device(&mut self, p: &Pkt) {
        let nodes = self.ring.gpu_nodes();
        let d = if p.x < 0 || p.y < 0 || nodes.is_empty() {
            Device::Unknown
        } else {
            let dev = (p.x as u32, p.y as u32);
            if nodes.contains(&dev) {
                Device::Same
            } else {
                Device::Other(dev.0, dev.1)
            }
        };
        let Some(c) = self.conn.as_mut() else { return };
        if c.device == d {
            return;
        }
        c.device = d;
        match d {
            Device::Same => {
                say!("the compositor renders on this GPU: the GPU-copy rung may be used")
            }
            Device::Other(a, b) => say!(
                "the compositor renders on DRM device {a}:{b}, not this GPU's ({nodes:?}): \
                 no GPU-copy frame on this connection; frames go through host memory"
            ),
            Device::Unknown => {}
        }
    }

    /// ★ The acknowledgement detector (X11 reports a refused import nowhere): native commits
    /// that no RELEASE acknowledged, [`DETECT_COMMITS`] of them over at least [`DETECT_MS`], back the
    /// rung off — loud once per trip, never permanent.
    fn detect(&mut self, now: u64) {
        let Some(c) = self.conn.as_mut() else { return };
        let due = !c.native_confirmed
            && c.native_unacked >= DETECT_COMMITS
            && c.native_unacked_since.is_some_and(|t| now >= t + DETECT_MS);
        if !due {
            return;
        }
        let ms = c.backoff_ms;
        c.backoff_until = Some(now + ms);
        c.backoff_ms = (ms * 2).min(BACKOFF_MAX_MS);
        let n = c.native_unacked;
        c.native_unacked = 0;
        c.native_unacked_since = None;
        self.counters.native_trips += 1;
        say!(
            "GPU-copy frames are not acknowledged ({n} committed, no RELEASE for {DETECT_MS} ms — \
             the compositor may not import them): backing off to the host rungs for {} s, then \
             trying again (trip {})",
            ms / 1000,
            self.counters.native_trips
        );
    }

    /// The close policy is the VMM's (`proto.h:73-91`); this keeps nvkvm-pv's repeat-ask message
    /// (`relay.c:1391-1453`). The VMM performs the request.
    fn close_policy(&mut self, now: u64, force: bool) {
        if force {
            say!(
                "the user chose to force the VM off{}",
                if self.powerdown.is_some() {
                    " after the guest ignored a powerdown"
                } else {
                    ""
                }
            );
            return;
        }
        match self.powerdown.as_mut() {
            Some((at, asks)) => {
                *asks += 1;
                say!(
                    "the guest has NOT responded to the powerdown requested {} s ago (asked {} \
                     times). It may be hung, or showing a dialog that blocks shutdown. Choose \
                     FORCE OFF THE VM to stop it anyway.",
                    now.saturating_sub(*at) / 1000,
                    asks
                );
            }
            None => {
                self.powerdown = Some((now, 1));
                say!(
                    "the user closed the display: requesting an ACPI powerdown (the guest decides \
                     what to do with it)"
                );
            }
        }
    }

    fn on_writable(&mut self, now: u64, host: &mut dyn Host) {
        let Some(phase) = self.conn.as_ref().map(|c| c.phase) else {
            return;
        };
        match phase {
            Phase::Replay(_) => self.continue_replay(now, host),
            Phase::Active => self.flush_owed(now, host),
            Phase::Hello => {}
        }
    }

    // ── deadlines, the watch and the timer ───────────────────────────────────────────────

    /// Every entry ends here: act on due deadlines, then (re)register the watch and the timer.
    fn finish(&mut self, now: u64, host: &mut dyn Host) {
        if self.stopped {
            return;
        }
        for _ in 0..4 {
            if self.conn.is_none() && self.retry_at.is_some_and(|t| t <= now) {
                self.attempt(now, host);
                continue;
            }
            let Some(c) = self.conn.as_ref() else { break };
            if c.phase != Phase::Active && c.handshake_until <= now {
                self.failed(
                    now,
                    host,
                    "the connect/HELLO/replay deadline expired (the broker stopped answering)",
                );
                continue;
            }
            if c.owed.is_some() && c.owed_at.is_some_and(|t| t <= now) {
                self.flush_owed(now, host);
                if let Some(c) = self.conn.as_mut()
                    && c.owed.is_some()
                {
                    c.owed_at = Some(now + OWED_MS);
                }
            }
            if let Some(c) = self.conn.as_mut()
                && !c.credit
                && c.credit_at.is_some_and(|t| t <= now)
            {
                c.credit = true;
                c.credit_at = None;
                self.counters.backstops += 1;
                self.progress(now, host);
            }
            if self.conn.is_some() && self.reclaim_deadline().is_some_and(|t| t <= now) {
                self.reclaim_overdue(now);
                self.progress(now, host);
            }
            self.detect(now);
            break;
        }
        // ★ the worker reads this once per frame: pack (or not) for the broker
        let want = self.native_allowed().is_some();
        if want != self.ring.want_vram() {
            self.ring.set_want_vram(want);
        }
        // the watch is set at connect and changed only here (and removed before a close)
        if let Some(c) = self.conn.as_mut() {
            let want = (true, c.want_write || c.owed.is_some());
            if want != c.watched {
                c.watched = want;
                host.watch(c.fd, want.0, want.1);
            }
        }
        let deadline = self.deadline();
        if deadline != self.timer {
            self.timer = deadline;
            host.timer(deadline);
        }
    }

    fn deadline(&self) -> Option<u64> {
        let Some(c) = self.conn.as_ref() else {
            return self.retry_at;
        };
        [
            (c.phase != Phase::Active).then_some(c.handshake_until),
            c.owed.and(c.owed_at),
            (!c.credit).then_some(c.credit_at).flatten(),
            self.reclaim_deadline(),
            c.native_unacked_since
                .filter(|_| !c.native_confirmed && c.native_unacked >= DETECT_COMMITS)
                .map(|t| t + DETECT_MS),
            c.backoff_until.filter(|t| *t > self.now_ms),
        ]
        .into_iter()
        .flatten()
        .min()
    }
}

fn verdict<S>(c: &Conn<S>, fourcc: u32, modifier: u64) -> Option<Verdict> {
    c.verdicts
        .iter()
        .find(|r| r.fourcc == fourcc && r.modifier == modifier)
        .map(|r| r.v)
}
