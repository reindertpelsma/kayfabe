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
//! - ★ **The guest's cursor as the host pointer** (`OWNER_RULINGS.md` §O, 2026-10-03): with a
//!   [`CursorShare`] attached ([`Relay::with_cursor`]) and a broker advertising `CAP_CURSOR`, the
//!   relay tracks the grab (`EV_GRAB`, and `F_GRABBED` on EVERY packet), publishes the mode the
//!   worker composes by, and in hover brings the broker to the guest's newest cursor with at most
//!   one `CMD_CURSOR` per entry ([`crate::cursor`]). Never under grab, never without the bit.
//! - ★ **nvkvm-pv `badf2d7`** (the broker's review fixes, wire unchanged; 2026-10-04): every "no"
//!   the broker volunteers is RECORDED, asked or not — it now sends `EV_FORMAT x=0` whenever an
//!   ATTACH is dropped at its format gate, and on X11 for both alpha twins of one refusal — in a
//!   table where a "no" for a pair the relay sends is never evicted by volunteered ones; the host
//!   dma-buf rungs get their own acknowledgement detector, because a dma-buf the broker cannot
//!   prove (without a readable `/proc/self/fdinfo`, every one) is dropped without a word on the
//!   wire; and every descriptor the relay sends is checked first, by the broker's own two tests in
//!   its order ([`kf_linux_raw::fd_carrier`]), so the broker never has to close one of ours on its
//!   helper thread. Its refusals are connection state, like the relay's.
//! - All per-connection knowledge lives in one [`Conn`] that is dropped on disconnect, so the
//!   "reconnect inherits partial state" class (nvkvm-pv audits B-2, S-11, RR-07) cannot be
//!   written.
//!
//! It is VMM-agnostic: socket I/O goes through [`Link`], fd-handler and timer registration
//! through [`Host`], and input comes back as [`Input`], which [`crate::InputPolicy`] delivers to
//! the VMM's [`crate::InputSink`]. Every entry takes
//! the time (`now_ms`), so the machine is deterministic under test. Every syscall is
//! non-blocking; nothing here waits.

use crate::cursor::{BrokerCursor, CursorMode, CursorOp, CursorShare, CursorWant, PointerAbs};
use crate::slots::{FrameRing, Kind, MAX_SLOTS, Take};
use crate::wire::{
    CAP_CURSOR, CAP_DEVICE, CAP_DMABUF, CAP_FOCUS_EVENTS, CAP_MODIFIERS, CAP_RELEASE, CLOSE_FORCE,
    CMD_ATTACH, CMD_F_SHM, CMD_SIZE, Cmd, CursorCmd, DEVICE_F_KNOWN, DEVICE_F_RENDER, EV_ABS,
    EV_BTN, EV_BYE, EV_CLIPBOARD, EV_CLOSE, EV_DEVICE, EV_FOCUS, EV_FORMAT, EV_FRAME, EV_GRAB,
    EV_HELLO, EV_KEY, EV_POINTER, EV_REL, EV_RELEASE, EV_SURFACE, EV_WHEEL, F_GRABBED, FOURCC_XR24,
    MOD_INVALID, MOD_LINEAR, PKT_SIZE, PROTO_VERSION, Pkt, fourcc_name,
};
use kf_linux_raw::Carrier;
use std::os::fd::BorrowedFd;
use std::os::unix::fs::FileExt;
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
/// Distinct (fourcc, modifier) verdicts remembered per connection. ⊘ 4 (`relay.c:92`) until
/// 2026-10-04: since nvkvm-pv `badf2d7` the broker VOLUNTEERS `x` = 0 for pairs the relay never
/// asked about — an ATTACH dropped at its format gate, and on X11 both alpha twins of one refusal
/// (`nvkvm_broker.c:768-821`, `:1735-1749`) — and it tells each pair ONCE per connection, so a
/// forgotten "no" is a pair the next frame goes into with nothing on the wire to say why. The
/// relay's OWN rows (pairs it asked about) are evicted only after every row the broker merely
/// volunteered, and its own "no"s last of all ([`remember`]).
const VERDICTS: usize = 16;
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

/// ★ §8.19 (2026-10-08): which of the VM's two pointing devices an event belongs to. The relay
/// decides it (the grab state is the broker's, mirrored on every packet); the VMM only maps it to
/// its own devices — the ABSOLUTE one (a tablet bound to the display) or the RELATIVE one (a
/// mouse).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pointer {
    /// The absolute pointer — every event while the broker is not grabbed.
    #[default]
    Absolute,
    /// The relative pointer — every pointer event while the broker is grabbed: motion, buttons
    /// and the wheel all come from ONE device, as from a real mouse.
    Relative,
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
        /// ★ §8.19: the device it belongs to (the relative one while grabbed).
        to: Pointer,
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
    /// One wheel detent per packet, by sign (the magnitude is not used, as before): `dy` +1 up
    /// (away from the user) / -1 down, `dx` +1 right / -1 left (★ 2026-10-08, §8.20: handed on;
    /// a VMM without a horizontal wheel — QEMU 10.2's virtio and USB pointers — ignores it).
    Wheel {
        /// Horizontal: -1, 0 or +1.
        dx: i32,
        /// Vertical: -1, 0 or +1.
        dy: i32,
        /// ★ §8.19: the device it belongs to (the relative one while grabbed).
        to: Pointer,
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
    /// ★ Host dma-buf (LINEAR, implicit) back-offs the same detector tripped: frames the broker
    /// dropped without a word (since nvkvm-pv `badf2d7`, any dma-buf it cannot prove, e.g.
    /// without a readable `/proc/self/fdinfo`).
    pub dmabuf_trips: u64,
    /// ★ Frames (and cursor images) NOT sent because their descriptor is not provably a memfd
    /// or a dma-buf ([`kf_linux_raw::fd_carrier`]) — the broker would refuse it and close it on
    /// a helper thread (`badf2d7`). A kayfabe defect whenever it is not zero.
    pub carrier_refused: u64,
    /// `EV_FORMAT` "no"s for a pair this connection never asked about, recorded (`badf2d7`).
    pub formats_unasked: u64,
    /// Descriptors sent although the check could not classify them (no `/proc` for the VMM).
    pub carrier_unchecked: u64,
    /// Classifications run ([`kf_linux_raw::fd_carrier`]); a backing that passed is not checked again.
    pub carrier_checks: u64,
    /// ★ §O: `CMD_CURSOR` SETs sent (an image or hot spot the broker did not hold) …
    pub cursor_sets: u64,
    /// … HIDEs …
    pub cursor_hides: u64,
    /// … SHOWs …
    pub cursor_shows: u64,
    /// … and SETs that could not be made (no memfd) — nothing was sent for them.
    pub cursor_refused: u64,
}

/// What the broker said about its compositor's device (`EV_DEVICE`, nvkvm-pv's header on
/// `broker-cursor-gpucopy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Device {
    /// The broker advertised `CAP_DEVICE` and its `EV_DEVICE` has not arrived yet (it follows the
    /// handshake's FRAME): no GPU-copy frame until it does.
    Pending,
    /// An older broker (no `CAP_DEVICE`), "the broker does not know" (`x` = 0), or an unresolved
    /// primary node that is not this GPU's: the explicit yes and the detector decide.
    Unknown,
    /// One of this GPU's DRM nodes.
    Same,
    /// Another device's RENDER node: no GPU-copy frame on this connection.
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
    /// This connection asked about the pair — one the relay itself sends (every rung asks before
    /// its first ATTACH), unlike a pair the broker merely volunteered a "no" for (an alpha twin).
    asked: bool,
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
    /// ★ Its COMMIT went (an acknowledgement detector counted it, when the broker promises
    /// RELEASEs) — so a frame the broker REFUSES by name is taken back out of that count.
    committed: bool,
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
    /// The broker window's last reported size and refresh (SURFACE is logged and handed on for a
    /// change only: some backends send it with every frame). ★ §8.16 (`display-max-fps`): the
    /// refresh is part of the key — a host monitor that changes only its rate re-authors the
    /// guest's monitor too (before, the size alone was compared and such a change was dropped).
    surface: Option<(i32, i32, u32)>,
    seq: u32,
    /// ★ The compositor's device (`EV_DEVICE`).
    device: Device,
    /// ★ The acknowledgement detector for the GPU-copy rung …
    native: Ack,
    /// … and for the host dma-buf rungs (LINEAR, implicit; nvkvm-pv `badf2d7`).
    dmabuf: Ack,
    /// ★ §O: the broker's grab, from `EV_GRAB` and from `F_GRABBED` on every packet.
    grabbed: bool,
    /// ★ §O: the cursor as this connection knows it.
    cursor: CursorConn,
    /// ★ §8.19: the last absolute position handed on (re-sent when a grab ends: the host's pointer
    /// was locked where it was, so the tablet goes back to it).
    last_abs: Option<(i32, i32, i32, i32)>,
    /// ★ §8.19: input received while grabbed, logged once a second (`grab_log`).
    grab_counts: GrabCounts,
}

/// ★ §8.19: what the broker sent while grabbed, in one logging interval — the instrument for "is an
/// absolute report, or a button on the absolute device, reaching the guest during mouse-look?".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct GrabCounts {
    since: u64,
    key: u32,
    btn: u32,
    wheel: u32,
    rel_packets: u32,
    rel_events: u32,
    abs_dropped: u32,
}

impl GrabCounts {
    fn any(&self) -> bool {
        self.key + self.btn + self.wheel + self.rel_packets + self.abs_dropped > 0
    }
}

/// ★ An acknowledgement detector for one class of dma-buf rungs. A broker that drops a dma-buf
/// ATTACH says so NOWHERE on the wire — X11 for a refused DRI3 import, and since nvkvm-pv
/// `badf2d7` both backends for any dma-buf whose identity the broker cannot prove (without a
/// readable `/proc/self/fdinfo` every one, `nvkvm_broker.c:1675-1689`; the frame is dropped and
/// the connection lives, `:2219-2231`) — so commits of the class that no RELEASE acknowledged,
/// [`DETECT_COMMITS`] of them over at least [`DETECT_MS`], back the class off for
/// [`BACKOFF_MIN_MS`] (doubling to [`BACKOFF_MAX_MS`]); a RELEASE naming a frame of the class
/// confirms it for the connection. Never permanent, and connection state: it dies with [`Conn`].
#[derive(Debug, Clone, Copy)]
struct Ack {
    /// A RELEASE named a frame of this class on this connection.
    acked: bool,
    /// Commits since the last confirmation, and when the first of them went.
    unacked: u32,
    unacked_since: Option<u64>,
    /// No frame of the class before this; the next back-off's length.
    backoff_until: Option<u64>,
    backoff_ms: u64,
}

impl Ack {
    const fn new() -> Ack {
        Ack {
            acked: false,
            unacked: 0,
            unacked_since: None,
            backoff_until: None,
            backoff_ms: BACKOFF_MIN_MS,
        }
    }

    /// A frame of the class was committed.
    fn committed(&mut self, now: u64) {
        if !self.acked {
            self.unacked += 1;
            self.unacked_since.get_or_insert(now);
        }
    }

    /// A RELEASE named a frame of the class: acknowledged, any back-off cleared. Whether that is
    /// news.
    fn released(&mut self) -> bool {
        let news = !self.acked;
        *self = Ack {
            acked: true,
            ..Ack::new()
        };
        news
    }

    /// Whether the class may be used at `now`.
    fn allows(&self, now: u64) -> bool {
        self.backoff_until.is_none_or(|t| now >= t)
    }

    /// When the detector decides next (a trip), if it has anything to decide.
    fn trip_at(&self) -> Option<u64> {
        self.unacked_since
            .filter(|_| !self.acked && self.unacked >= DETECT_COMMITS)
            .map(|t| t + DETECT_MS)
    }

    /// Trip, when due at `now`: the commits it counted and the back-off's length.
    fn trip(&mut self, now: u64) -> Option<(u32, u64)> {
        if !self.trip_at().is_some_and(|t| now >= t) {
            return None;
        }
        let n = self.unacked;
        Some((n, self.back_off(now)))
    }

    /// Back the class off from `now` (a trip, or a descriptor of the class the broker could not
    /// prove): its length, doubled for the next one.
    fn back_off(&mut self, now: u64) -> u64 {
        let ms = self.backoff_ms;
        self.backoff_until = Some(now + ms);
        self.backoff_ms = (ms * 2).min(BACKOFF_MAX_MS);
        self.unacked = 0;
        self.unacked_since = None;
        ms
    }

    /// ★ A counted commit was REFUSED by name (`EV_FORMAT x=0` for its pair, its frame reclaimed):
    /// it is an answer, not a silent drop, so it no longer counts toward a trip. (⊘ The review of
    /// 2026-10-04: the first frames of a connection race the relay's own question, and their drop
    /// at the broker's format gate tripped a back-off that blamed `/proc/self/fdinfo`.) The first
    /// remaining commit's time is not known; the earlier one is kept, which can only trip sooner.
    fn uncount(&mut self) {
        if self.acked || self.unacked == 0 {
            return;
        }
        self.unacked -= 1;
        if self.unacked == 0 {
            self.unacked_since = None;
        }
    }
}

/// ★ §O: the guest's cursor on one connection — what the broker holds, the newest want taken
/// from the [`CursorShare`], and whether a command waits for the socket.
#[derive(Debug, Default)]
struct CursorConn {
    broker: BrokerCursor,
    want: Option<CursorWant>,
    /// The share's generation last taken.
    seen: u64,
    /// The last command met `EAGAIN`: retried at the next entry (the watch asks for writability).
    owed: bool,
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
    /// ★ §O: the worker ↔ relay share for the guest's cursor; `None` = cursors stay composed.
    cursor: Option<Arc<CursorShare>>,
    /// Why [`Relay::attach`] last refused a descriptor ([`carried`]), for the refusal's log line.
    carrier_refusal: Option<String>,
    /// ★ Per slot and rung: the identity of the descriptor last PROVEN a memfd or a dma-buf — the
    /// check reads `/proc/self/fdinfo` once per backing, not once per frame (the review,
    /// 2026-10-04). A descriptor's kind cannot change while it is open, and a new backing has a new
    /// identity, so it is checked again.
    carrier_ok: [[Option<u64>; 4]; MAX_SLOTS],
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
            cursor: None,
            carrier_refusal: None,
            carrier_ok: [[None; 4]; MAX_SLOTS],
        }
    }

    /// ★ §O: give the relay the cursor share it and the display worker meet in. Without it a
    /// broker's `CAP_CURSOR` is ignored and the cursor stays composed.
    #[must_use]
    pub fn with_cursor(mut self, share: Arc<CursorShare>) -> Relay<L> {
        self.cursor = Some(share);
        self
    }

    /// ★ §O: the cursor mode as the relay last published it ([`CursorMode::Off`] without a share).
    #[must_use]
    pub fn cursor_mode(&self) -> CursorMode {
        self.cursor.as_ref().map_or(CursorMode::Off, |s| s.mode())
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
            "broker[up={} sent={} gpucopy={} gpucopy_backoffs={} dmabuf_trips={} carrier_refused={} carrier_unchecked={} formats_unasked={} dropped={} uncommitted={} recovered={} releases={} unknown_releases={} reclaims={} blocked={} backstops={} reconnects={} failed_attempts={} peer_refused={} cursor={} cursor_sets={} cursor_hides={} cursor_shows={} cursor_refused={}]",
            u8::from(self.active()),
            c.sent,
            c.native,
            c.native_trips,
            c.dmabuf_trips,
            c.carrier_refused,
            c.carrier_unchecked,
            c.formats_unasked,
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
            c.peer_refused,
            self.cursor_mode().name(),
            c.cursor_sets,
            c.cursor_hides,
            c.cursor_shows,
            c.cursor_refused
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
            native: Ack::new(),
            dmabuf: Ack::new(),
            grabbed: false,
            cursor: CursorConn::default(),
            last_abs: None,
            grab_counts: GrabCounts::default(),
        });
        host.watch(fd, true, false);
    }

    /// Unwatch, then close; give back every held frame except the latest commit, which becomes
    /// the next connection's replay frame when nothing newer is ready.
    fn close(&mut self, host: &mut dyn Host) {
        let Some(c) = self.conn.take() else { return };
        // no connection, no GPU-copy demand: the worker stops packing at its next frame
        self.ring.set_want_vram(false);
        // ★ §O: and no host cursor — the worker composes the cursor again from its next frame
        if let Some(s) = &self.cursor
            && s.set_mode(CursorMode::Off)
        {
            say!("guest cursor: composed into the frame (no broker)");
        }
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
        let fd = slot_fd.and_then(|j| ring_fd(&self.ring, j, rung));
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
            && !matches!(c.device, Device::Other(..) | Device::Pending)
            && c.native.allows(self.now_ms);
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
        // ★ the host dma-buf rungs, unless their detector backs off (`badf2d7`: a dma-buf the
        // broker cannot prove is dropped without a word)
        if fds.dmabuf().is_some() && c.dmabuf.allows(self.now_ms) {
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
            remember(c, fourcc, modifier, Verdict::Asked, true);
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
            committed: false,
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
        let carrier = self.carrier_refusal.take();
        if loud(n) {
            if let Some(why) = carrier {
                say!(
                    "REFUSED frame slot {j}: {why} — the broker accepts only a memfd or a dma-buf \
                     it can prove, and closes any other on a helper thread ({n} so far)"
                );
            } else if self.ring.broker_backed(j) {
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

    /// The ATTACH for `slot` on `rung` (a seq is spent).
    fn attach_cmd(&mut self, slot: usize, rung: Rung, modifier: u64) -> Option<Cmd> {
        let g = self.ring.geometry(slot);
        let stride = if rung == Rung::Native {
            self.ring.vram_geometry(slot).stride
        } else {
            g.stride
        };
        let c = self.conn.as_mut()?;
        let seq = c.seq;
        c.seq = c.seq.wrapping_add(1);
        Some(Cmd {
            ty: CMD_ATTACH,
            flags: if rung == Rung::Shm { CMD_F_SHM } else { 0 },
            width: g.width,
            height: g.height,
            stride,
            offset: 0,
            fourcc: g.fourcc,
            modifier,
            seq,
        })
    }

    /// The rung's line, once per change on a connection — said only once a frame WENT on it (⊘ the
    /// review, 2026-10-04: it was said before the descriptor check, so a refused frame had already
    /// announced a rung that carried nothing).
    fn log_rung(&mut self, rung: Rung, modifier: u64) {
        let Some(c) = self.conn.as_mut() else { return };
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
    ///
    /// ★ `badf2d7`: only a descriptor the broker can prove — never one it must close off its main
    /// thread. The check runs BEFORE anything is spent on the frame (a seq, the rung's line). ⊘ The
    /// review of 2026-10-04: a refusal used to refuse the FRAME on the rung [`Self::choose`] picked,
    /// so a GPU-copy descriptor that failed the check would have been picked again for every frame
    /// — a permanently black broker display with nothing tripping. Now a refused dma-buf rung
    /// (the GPU copy, or the host dma-bufs) backs off for this connection, exactly as an
    /// unacknowledged one does, and the same frame goes on the next rung; only a refused `F_SHM`
    /// memfd, the last rung, refuses the frame.
    fn attach(&mut self, now: u64, slot: usize) -> Option<Sent> {
        let mut tries = 0;
        let mut refused: Option<String> = None;
        let (rung, modifier) = loop {
            let Some((rung, modifier)) = self.choose(slot) else {
                // a frame only the refused rung holds (a VRAM-only GPU copy): refused, by name
                if let Some(why) = refused {
                    self.carrier_refusal = Some(format!("{why}, and no other rung holds it"));
                }
                return None;
            };
            let Err(why) = self.check_carrier(slot, rung) else {
                break (rung, modifier);
            };
            self.counters.carrier_refused += 1;
            tries += 1;
            let what = format!("its {rung:?} descriptor {why}");
            let backoff = if tries < 3 {
                self.conn.as_mut().and_then(|c| match rung {
                    Rung::Native => Some(c.native.back_off(now)),
                    Rung::Linear | Rung::Implicit => Some(c.dmabuf.back_off(now)),
                    Rung::Shm => None,
                })
            } else {
                None
            };
            let Some(ms) = backoff else {
                self.carrier_refusal = Some(what);
                return None;
            };
            refused = Some(what);
            let n = self.counters.carrier_refused;
            if loud(n) {
                say!(
                    "a {rung:?} frame descriptor {why} — the broker accepts only a memfd or a \
                     dma-buf it can prove; that rung backs off for {} s on this connection and the \
                     frame takes the next ({n} refused so far)",
                    ms / 1000
                );
            }
        };
        let cmd = self.attach_cmd(slot, rung, modifier)?;
        let r = self.send(&cmd, Some(slot), rung);
        if r == Sent::Done {
            self.log_rung(rung, modifier);
            self.held[slot] = Some(Held {
                sent_id: self.sent_id(slot, rung),
                rung,
                fourcc: cmd.fourcc,
                modifier: cmd.modifier,
                at_ms: now,
                commit_no: self.commits,
                released: false,
                superseded: false,
                committed: false,
            });
        }
        Some(r)
    }

    /// ★ Whether `slot`'s descriptor for `rung` may be sent ([`carried`]): `Err` names why not. A
    /// descriptor that passed once is not checked again while the slot keeps it
    /// ([`Relay::carrier_ok`]); one the check cannot classify is sent, counted and logged.
    fn check_carrier(&mut self, slot: usize, rung: Rung) -> Result<(), String> {
        let k = match rung {
            Rung::Native => 0,
            Rung::Linear => 1,
            Rung::Implicit => 2,
            Rung::Shm => 3,
        };
        let id = self.sent_id(slot, rung);
        if id.is_some() && self.carrier_ok.get(slot).map(|r| r[k]) == Some(id) {
            return Ok(());
        }
        let Some(fd) = ring_fd(&self.ring, slot, rung) else {
            return Err("has no descriptor".into());
        };
        self.counters.carrier_checks += 1;
        match carried(kf_linux_raw::fd_carrier(fd), rung == Rung::Shm)? {
            None => {
                if let Some(r) = self.carrier_ok.get_mut(slot) {
                    r[k] = id;
                }
            }
            Some(e) => self.unclassified(&e),
        }
        Ok(())
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
            let rung = self.held[slot].map(|h| h.rung);
            if rung == Some(Rung::Native) {
                self.counters.native += 1;
            }
            // ★ the acknowledgement detectors count commits only where a RELEASE is promised
            if let Some(c) = self.conn.as_mut()
                && c.caps & CAP_RELEASE != 0
            {
                match rung {
                    Some(Rung::Native) => c.native.committed(now),
                    Some(Rung::Linear | Rung::Implicit) => c.dmabuf.committed(now),
                    Some(Rung::Shm) | None => {}
                }
            }
            if let Some(h) = self.held[slot].as_mut() {
                h.at_ms = now;
                h.commit_no = self.commits;
                h.committed = true;
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
        // ★ §O: the grab is mirrored on EVERY packet (`proto.h`: "a client can never disagree with
        // the broker about grab state"); EV_GRAB's own `x` is the edge it announces
        let mut was_grabbed = false;
        if let Some(c) = self.conn.as_mut() {
            was_grabbed = c.grabbed;
            c.grabbed = if p.ty == EV_GRAB {
                p.x != 0
            } else {
                p.flags & F_GRABBED != 0
            };
        }
        let grabbed = self.conn.as_ref().is_some_and(|c| c.grabbed);
        self.grab_count(now, p, grabbed, was_grabbed);
        // ★ §8.19: while grabbed every pointer event belongs to the RELATIVE device — motion,
        // buttons and the wheel from one device, as from a real mouse. A button on the absolute
        // device made the guest's X server switch its pointer to that slave, whose last position
        // is the tablet's stale one: the snap back a warp-to-centre game sees as a rejected move.
        let to = if grabbed {
            Pointer::Relative
        } else {
            Pointer::Absolute
        };
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
                    to,
                },
            ),
            // ★ §8.19: no absolute report is generated while grabbed (the broker sends none —
            // `nb_sink_abs`; one that does is dropped here and counted)
            EV_ABS if grabbed => {}
            EV_ABS if p.w0 > 0 && p.w1 > 0 => {
                let w = i32::try_from(p.w0.min(1 << 20)).unwrap_or(1);
                let h = i32::try_from(p.w1.min(1 << 20)).unwrap_or(1);
                let (x, y) = (p.x.clamp(0, w - 1), p.y.clamp(0, h - 1));
                if let Some(c) = self.conn.as_mut() {
                    c.last_abs = Some((x, y, w, h));
                }
                // ★ §O: the pointer the guest will move its cursor to — the worker derives the
                // hot spot NVKMS does not program from it (`crate::cursor::HotTracker`)
                if let Some(s) = &self.cursor {
                    s.note_abs(PointerAbs {
                        x,
                        y,
                        w: w.unsigned_abs(),
                        h: h.unsigned_abs(),
                    });
                }
                emit(out, Input::Abs { x, y, w, h });
            }
            EV_REL => {
                if let Some(Input::Rel { dx, dy }) = out.last_mut() {
                    *dx = dx.saturating_add(p.x);
                    *dy = dy.saturating_add(p.y);
                } else {
                    emit(out, Input::Rel { dx: p.x, dy: p.y });
                }
            }
            // `proto.h`: x = vertical detents, y = horizontal
            EV_WHEEL if p.x != 0 || p.y != 0 => emit(
                out,
                Input::Wheel {
                    dx: p.y.signum(),
                    dy: p.x.signum(),
                    to,
                },
            ),
            EV_GRAB => {
                say!("grab {}", if p.x != 0 { "ON" } else { "off" });
                emit(out, Input::Grab(p.x != 0));
                // ★ §8.19: the grab ended — the absolute device is put back where the host's
                // pointer is: where it was locked, i.e. the last position before the grab
                if p.x == 0
                    && let Some((x, y, w, h)) = self.conn.as_ref().and_then(|c| c.last_abs)
                {
                    emit(out, Input::Abs { x, y, w, h });
                }
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
                    && c.surface != Some((w, h, p.w0))
                {
                    c.surface = Some((w, h, p.w0));
                    say!("broker window is now {w}x{h} at {} mHz", p.w0);
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

    /// ★ §8.19: count what arrives while grabbed and log it once a second (and at the grab's end):
    /// keys, buttons and wheel ticks (all handed to the relative device), REL packets, and any
    /// absolute report the broker sent anyway (dropped).
    fn grab_count(&mut self, now: u64, p: &Pkt, grabbed: bool, was_grabbed: bool) {
        let Some(c) = self.conn.as_mut() else {
            return;
        };
        let g = &mut c.grab_counts;
        if grabbed {
            if g.since == 0 {
                g.since = now.max(1);
            }
            match p.ty {
                EV_KEY => g.key += 1,
                EV_BTN => g.btn += 1,
                EV_WHEEL => g.wheel += 1,
                EV_REL => g.rel_packets += 1,
                EV_ABS => g.abs_dropped += 1,
                _ => {}
            }
        }
        let due = grabbed && now.saturating_sub(g.since) >= 1000;
        let ended = was_grabbed && !grabbed;
        if (due || ended) && g.any() {
            let s = *g;
            say!(
                "input while grabbed ({} ms): {} key, {} button and {} wheel event(s) to the \
                 relative pointer, {} REL packet(s), {} absolute report(s) dropped{}",
                now.saturating_sub(s.since),
                s.key,
                s.btn,
                s.wheel,
                s.rel_packets,
                s.abs_dropped,
                if ended { " — grab ended" } else { "" }
            );
        }
        if due || ended {
            *g = GrabCounts {
                since: if grabbed { now.max(1) } else { 0 },
                ..GrabCounts::default()
            };
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
        // ⊘ CORRECTED 2026-10-04 (box run brkF1, then the review): "no /dev/udmabuf here" was said
        // whenever no slot held a dma-buf yet — and at the first connection, before the guest's
        // first frame, no slot holds anything. Only installed slots that ALL lack one say so (the
        // device's own line at start says whether /dev/udmabuf opened).
        let installed = (0..self.ring.slots()).filter_map(|j| self.ring.fds(j));
        let (mut slots, mut dmabufs) = (0usize, 0usize);
        for f in installed {
            slots += 1;
            dmabufs += usize::from(f.dmabuf().is_some());
        }
        say!(
            "connected to {}, capabilities {:#x}{}",
            self.cfg.path.display(),
            p.w1,
            if slots > 0 && dmabufs == 0 {
                " — no frame slot carries a dma-buf (no /dev/udmabuf, or UDMABUF_CREATE refused), \
                 so frames go as shared memory"
            } else {
                ""
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
            c.grabbed = p.flags & F_GRABBED != 0;
            if p.w1 & CAP_DEVICE != 0 {
                c.device = Device::Pending;
            }
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
        // ★ a RELEASE names only a buffer the compositor imported: its class is acknowledged
        let rung = self.held[j].map(|h| h.rung);
        if let Some(c) = self.conn.as_mut() {
            match rung {
                Some(Rung::Native) => {
                    if c.native.released() {
                        say!("the display imported a GPU-copy frame (its RELEASE came back)");
                    }
                }
                Some(Rung::Linear | Rung::Implicit) => {
                    c.dmabuf.released();
                }
                Some(Rung::Shm) | None => {}
            }
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

    /// ★ `EV_FORMAT`: the answer to a `QUERY_FORMAT` — or, since nvkvm-pv `badf2d7`, a "no" the
    /// broker VOLUNTEERS: when the display refuses an import it had advertised, and whenever an
    /// ATTACH is dropped at its format gate, for the pair that ATTACH named (once per pair per
    /// connection, possibly a pair this connection never asked about), on X11 for BOTH alpha
    /// twins of one refusal (`nvkvm_broker.c:768-821`, `:1735-1749`). Every "no" is recorded — a
    /// later one always wins, asked or not — and reclaims the frames attached under its pair; a
    /// "yes" counts only as the answer to a question this connection asked (never an upgrade).
    /// The record is connection state ([`Conn`]): the broker forgets its refusals at detach
    /// (`badf2d7`), and so does the relay.
    fn format(&mut self, now: u64, host: &mut dyn Host, p: &Pkt) {
        let fourcc = u32::from_le_bytes(p.y.to_le_bytes());
        let modifier = p.wide();
        let Some(c) = self.conn.as_mut() else { return };
        let last = c.last_commit;
        let was = verdict(c, fourcc, modifier);
        let v = match (was, p.x != 0) {
            (_, false) => Verdict::No, // ★ a later "no" always wins
            (Some(Verdict::Asked), true) => Verdict::Yes,
            (Some(w), true) => w,
            (None, true) => {
                say!(
                    "unasked EV_FORMAT yes for {} modifier {modifier:#x}, ignored (a yes is \
                     never an upgrade)",
                    fourcc_name(fourcc)
                );
                return;
            }
        };
        if was != Some(v) {
            remember(c, fourcc, modifier, v, false);
            if was.is_none() {
                self.counters.formats_unasked += 1;
            }
            let n = self.counters.formats_unasked;
            if was.is_some() || loud(n) {
                say!(
                    "the display {} show {} modifier {modifier:#018x}{}",
                    if v == Verdict::Yes { "CAN" } else { "CANNOT" },
                    fourcc_name(fourcc),
                    if was.is_none() {
                        " (unasked: the broker dropped an ATTACH in this pair, or refused its \
                         alpha twin)"
                    } else {
                        ""
                    }
                );
            }
        }
        if p.x != 0 {
            return;
        }
        // ★ fix (a): free what that pair orphaned — the broker failed to import them
        let mut credit = false;
        for j in 0..MAX_SLOTS {
            let Some(h) = self.held[j] else { continue };
            if h.sent_id.is_some()
                && h.rung != Rung::Shm
                && h.fourcc == fourcc
                && h.modifier == modifier
            {
                // ★ refused BY NAME: not a silent drop for the acknowledgement detector
                if h.committed
                    && let Some(c) = self.conn.as_mut()
                {
                    match h.rung {
                        Rung::Native => c.native.uncount(),
                        Rung::Linear | Rung::Implicit => c.dmabuf.uncount(),
                        Rung::Shm => {}
                    }
                }
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
    ///
    /// `x` = `DEVICE_F_*`, `w0`:`w1` = major:minor. A render node of another device is
    /// [`Device::Other`]; an unresolved node (KNOWN without RENDER — the header says "compare with
    /// care": it may be a primary node sysfs could not map) counts only when it IS one of this
    /// GPU's nodes, and is [`Device::Unknown`] otherwise, so a node the broker could not resolve
    /// never shuts the rung off by itself.
    fn device(&mut self, p: &Pkt) {
        let nodes = self.ring.gpu_nodes();
        let known = p.x & DEVICE_F_KNOWN != 0;
        let render = p.x & DEVICE_F_RENDER != 0;
        let dev = (p.w0, p.w1);
        let d = if !known || nodes.is_empty() {
            Device::Unknown
        } else if nodes.contains(&dev) {
            Device::Same
        } else if render {
            Device::Other(dev.0, dev.1)
        } else {
            say!(
                "the compositor's DRM device {}:{} is an unresolved node, not one of this GPU's \
                 ({nodes:?}): it does not decide the GPU-copy rung",
                dev.0,
                dev.1
            );
            Device::Unknown
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
            Device::Unknown | Device::Pending => {}
        }
    }

    /// ★ The acknowledgement detector (X11 reports a refused import nowhere): native commits
    /// that no RELEASE acknowledged, [`DETECT_COMMITS`] of them over at least [`DETECT_MS`], back the
    /// rung off — loud once per trip, never permanent.
    fn detect(&mut self, now: u64) {
        let Some(c) = self.conn.as_mut() else { return };
        let native = c.native.trip(now);
        let dmabuf = c.dmabuf.trip(now);
        if let Some((n, ms)) = native {
            self.counters.native_trips += 1;
            say!(
                "GPU-copy frames are not acknowledged ({n} committed, no RELEASE for {DETECT_MS} ms — \
                 the compositor may not import them): backing off to the host rungs for {} s, then \
                 trying again (trip {})",
                ms / 1000,
                self.counters.native_trips
            );
        }
        if let Some((n, ms)) = dmabuf {
            self.counters.dmabuf_trips += 1;
            say!(
                "dma-buf frames are not acknowledged ({n} committed, no RELEASE for {DETECT_MS} ms): \
                 the broker drops a dma-buf it cannot prove without a word on the wire — it needs a \
                 readable /proc/self/fdinfo (its own log says why) — so frames go as shared memory \
                 for {} s, then the dma-buf rung is tried again (trip {})",
                ms / 1000,
                self.counters.dmabuf_trips
            );
        }
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
        // ★ §O: the cursor mode for the worker, and at most one cursor command
        self.cursor_sync(now, host);
        // the watch is set at connect and changed only here (and removed before a close)
        if let Some(c) = self.conn.as_mut() {
            let want = (true, c.want_write || c.owed.is_some() || c.cursor.owed);
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

    /// ★ §O (`crate::cursor`): publish the mode the worker composes by — [`CursorMode::Hover`]
    /// for an ACTIVE `CAP_CURSOR` broker that is not grabbed, [`CursorMode::Grabbed`] when it is,
    /// [`CursorMode::Off`] otherwise — take the guest's newest cursor, and in hover send at most
    /// ONE command that brings the broker to it. Under grab nothing is sent: the broker hides its
    /// image itself and puts it back when the grab ends, and the first entry after that sends
    /// whatever changed meanwhile.
    fn cursor_sync(&mut self, now: u64, host: &mut dyn Host) {
        let Some(share) = self.cursor.clone() else {
            return;
        };
        let mode = match self.conn.as_ref() {
            Some(c) if c.phase == Phase::Active && c.caps & CAP_CURSOR != 0 => {
                if c.grabbed {
                    CursorMode::Grabbed
                } else {
                    CursorMode::Hover
                }
            }
            _ => CursorMode::Off,
        };
        if share.set_mode(mode) {
            say!(
                "guest cursor: {}",
                match mode {
                    CursorMode::Hover => "the HOST pointer shows it (hover; not composed)",
                    CursorMode::Grabbed =>
                        "composed into the frame (grabbed; the broker hides its own image)",
                    CursorMode::Off => "composed into the frame",
                }
            );
        }
        if mode == CursorMode::Off {
            return;
        }
        let Some(c) = self.conn.as_mut() else { return };
        if let Some((g, w)) = share.take_newer(c.cursor.seen) {
            c.cursor.seen = g;
            c.cursor.want = Some(w);
        }
        if mode != CursorMode::Hover {
            c.cursor.owed = false;
            return;
        }
        let Some(want) = c.cursor.want.clone() else {
            return;
        };
        let Some(op) = c.cursor.broker.next_op(&want) else {
            c.cursor.owed = false;
            return;
        };
        match self.send_cursor(op, &want) {
            Ok(Sent::Done) => {
                if let Some(c) = self.conn.as_mut() {
                    c.cursor.broker.applied(op, &want);
                    c.cursor.owed = false;
                }
                let k = &mut self.counters;
                match op {
                    CursorOp::Set => {
                        k.cursor_sets += 1;
                        if loud(k.cursor_sets)
                            && let CursorWant::Image(i) = &want
                        {
                            say!(
                                "guest cursor image {}x{} hot {},{} sent ({} so far)",
                                i.width(),
                                i.height(),
                                i.hot().0,
                                i.hot().1,
                                k.cursor_sets
                            );
                        }
                    }
                    CursorOp::Hide => k.cursor_hides += 1,
                    CursorOp::Show => k.cursor_shows += 1,
                }
            }
            Ok(Sent::Full) => {
                if let Some(c) = self.conn.as_mut() {
                    c.cursor.owed = true;
                }
            }
            Ok(Sent::Failed(e)) => {
                self.lost(
                    now,
                    host,
                    &format!("the broker socket failed on a cursor command: {e}"),
                );
            }
            Err(e) => {
                self.counters.cursor_refused += 1;
                let n = self.counters.cursor_refused;
                if loud(n) {
                    say!("guest cursor NOT sent: {e} ({n} so far)");
                }
            }
        }
    }

    /// A descriptor the check could not classify went anyway (see [`carried`]): counted, logged
    /// at a bounded rate.
    fn unclassified(&mut self, e: &str) {
        self.counters.carrier_unchecked += 1;
        let n = self.counters.carrier_unchecked;
        if loud(n) {
            say!(
                "a frame or cursor descriptor could not be classified ({e}) and was sent unchecked — \
                 it is a memfd or a dma-buf by construction; is /proc unreadable to the VMM \
                 (a chroot without /proc)? ({n} so far)"
            );
        }
    }

    /// One `CMD_CURSOR`. A SET's image goes in a sealed memfd made here, filled, sent with the
    /// record and CLOSED when this returns: the broker `pread`s its own copy, and nothing of the
    /// guest's is ever handed over (the pixels are kayfabe's copy, `crate::cursor`).
    ///
    /// # Errors
    /// The SET could not be made (the memfd, its write, a bound) — nothing was sent.
    fn send_cursor(&mut self, op: CursorOp, want: &CursorWant) -> Result<Sent, String> {
        let Some(c) = self.conn.as_ref() else {
            return Ok(Sent::Failed("no connection".into()));
        };
        match (op, want) {
            (CursorOp::Hide, _) => Ok(self.link.send(&c.sock, &CursorCmd::hide().encode(), None)),
            (CursorOp::Show, _) => Ok(self.link.send(&c.sock, &CursorCmd::show().encode(), None)),
            (CursorOp::Set, CursorWant::Image(i)) => {
                let bytes = i.pixels().len() as u64;
                let cmd = CursorCmd::set(i.width(), i.height(), i.stride(), 0, i.hot(), bytes)
                    .ok_or_else(|| {
                        format!(
                            "a {}x{} cursor is outside the broker's bounds",
                            i.width(),
                            i.height()
                        )
                    })?;
                let ram = kf_linux_raw::SharedRam::create_named(c"kayfabe-cursor", bytes)
                    .map_err(|e| format!("the cursor memfd: {e}"))?;
                let f = std::fs::File::from(
                    ram.dup_for_export()
                        .map_err(|e| format!("the cursor memfd: {e}"))?,
                );
                f.write_all_at(i.pixels(), 0)
                    .map_err(|e| format!("writing the cursor memfd: {e}"))?;
                drop(f);
                match carried(kf_linux_raw::fd_carrier(ram.as_backing_fd()), true) {
                    Ok(None) => {}
                    Ok(Some(e)) => self.unclassified(&e),
                    Err(why) => {
                        self.counters.carrier_refused += 1;
                        return Err(format!("the cursor memfd {why}"));
                    }
                }
                let c = self.conn.as_ref().ok_or("no connection")?;
                Ok(self
                    .link
                    .send(&c.sock, &cmd.encode(), Some(ram.as_backing_fd())))
            }
            (CursorOp::Set, _) => Err("a SET with no image".into()),
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
            c.native.trip_at(),
            c.dmabuf.trip_at(),
            c.native.backoff_until.filter(|t| *t > self.now_ms),
            c.dmabuf.backoff_until.filter(|t| *t > self.now_ms),
        ]
        .into_iter()
        .flatten()
        .min()
    }
}

/// The descriptor `rung` sends for slot `j` of `ring`.
fn ring_fd(ring: &FrameRing, j: usize, rung: Rung) -> Option<BorrowedFd<'_>> {
    match rung {
        Rung::Native => ring.vram(j).map(crate::slots::VramFds::fd),
        Rung::Shm => ring.fds(j).map(crate::SlotFds::memfd),
        Rung::Linear | Rung::Implicit => ring.fds(j).and_then(crate::SlotFds::dmabuf),
    }
}

/// ★ nvkvm-pv `badf2d7`: the broker accepts only a descriptor it can PROVE to be shmem or a
/// dma-buf, and closes any other on a helper thread (`nb_fd_drop`, `nvkvm_broker.c:1487-1520`).
/// So every descriptor this relay sends is checked first, by the broker's own two tests in its
/// order ([`kf_linux_raw::fd_carrier`]: `F_GET_SEALS`, then `/proc/self/fdinfo`'s `exp_name:`;
/// never an `fstatfs` on an unproven descriptor) — a memfd or a dma-buf, and an `F_SHM` frame's
/// (or a cursor's) memfd sealed `F_SEAL_SHRINK`, which the broker requires of it
/// (`:1640-1668`). The ring's descriptors are memfds and dma-bufs by construction; this is the
/// one place that says so on the wire's side. `Ok(None)`: send it; `Err`: refuse it, naming why.
///
/// ⊘ `Ok(Some(why))` — a descriptor the check could not CLASSIFY (`/proc/self/fdinfo` unreadable
/// to the VMM, as in a `-run-with chroot=` without `/proc`; a memfd needs no `/proc`, a dma-buf
/// does) is sent anyway, and the caller logs it: the check is a run-time statement of what the
/// descriptors are by construction, and refusing every dma-buf for the VMM's own missing `/proc`
/// would black out a display the broker (with its own `/proc`) can show.
fn carried(
    c: Result<Carrier, kf_linux_raw::RawError>,
    shm: bool,
) -> Result<Option<String>, String> {
    match c {
        Ok(c @ Carrier::Shmem { .. }) if shm && !c.sealed_against_shrinking() => {
            Err("is a memfd not sealed F_SEAL_SHRINK".into())
        }
        Ok(Carrier::DmaBuf) if shm => Err("is a dma-buf, not a memfd".into()),
        Ok(Carrier::Shmem { .. } | Carrier::DmaBuf) => Ok(None),
        Ok(Carrier::Neither) => Err("is neither a memfd nor a dma-buf".into()),
        Err(e) => Ok(Some(e.to_string())),
    }
}

/// ★ Record `v` for `(fourcc, modifier)` on connection `c` (`asked`: this connection asked about
/// it): update its row, or add one. A full table ([`VERDICTS`]) gives up, in this order, its
/// oldest row the relay never asked about (one the broker volunteered — say an alpha twin the
/// relay does not send — whatever its verdict), then its oldest asked row that is not a "no" (an
/// evicted question is simply asked again), and only then its oldest "no": the relay asks about at
/// most a few pairs per connection, so the broker's volunteered rows can never push out one of
/// its own — not a "no" (a pair the broker will not mention again on this connection) and not a
/// yes (⊘ the review of 2026-10-04: the table used to drop its oldest non-"no" FIRST, so sixteen
/// volunteered "no"s evicted the relay's recorded yes for the GPU-copy pair, and the rung, the
/// worker's pack and the "CAN show" line flapped frame by frame).
fn remember<S>(c: &mut Conn<S>, fourcc: u32, modifier: u64, v: Verdict, asked: bool) {
    if let Some(r) = c
        .verdicts
        .iter_mut()
        .find(|r| r.fourcc == fourcc && r.modifier == modifier)
    {
        r.v = v;
        r.asked |= asked;
        return;
    }
    if c.verdicts.len() >= VERDICTS {
        let rows = &c.verdicts;
        let k = rows
            .iter()
            .position(|r| !r.asked)
            .or_else(|| rows.iter().position(|r| r.v != Verdict::No))
            .unwrap_or(0);
        c.verdicts.remove(k);
    }
    c.verdicts.push(VerdictRow {
        fourcc,
        modifier,
        v,
        asked,
    });
}

fn verdict<S>(c: &Conn<S>, fourcc: u32, modifier: u64) -> Option<Verdict> {
    c.verdicts
        .iter()
        .find(|r| r.fourcc == fourcc && r.modifier == modifier)
        .map(|r| r.v)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★ The descriptor rule as a table: what is refused, what goes, and that a descriptor the VMM
    /// cannot classify (no `/proc` for it) still goes — the broker, with its own `/proc`, decides.
    #[test]
    fn the_descriptor_rule_refuses_only_what_it_can_name() {
        let sealed = Carrier::Shmem {
            seals: libc_seal_shrink(),
        };
        let unsealed = Carrier::Shmem { seals: 1 };
        let no_proc = || {
            Err(kf_linux_raw::RawError::Syscall {
                call: "read /proc/self/fdinfo (fd_carrier)",
                errno: Some(2),
            })
        };
        assert_eq!(carried(Ok(sealed), true), Ok(None), "an F_SHM memfd");
        assert!(
            carried(Ok(unsealed), true).is_err(),
            "F_SHM without F_SEAL_SHRINK"
        );
        assert!(
            carried(Ok(Carrier::DmaBuf), true).is_err(),
            "a dma-buf as F_SHM"
        );
        assert_eq!(carried(Ok(Carrier::DmaBuf), false), Ok(None), "a dma-buf");
        assert_eq!(carried(Ok(unsealed), false), Ok(None), "a memfd stand-in");
        assert!(carried(Ok(Carrier::Neither), false).is_err(), "neither");
        assert!(carried(Ok(Carrier::Neither), true).is_err(), "neither");
        assert!(
            matches!(carried(no_proc(), false), Ok(Some(_))),
            "unclassified: sent"
        );
    }

    /// `F_SEAL_SHRINK` (`include/uapi/linux/fcntl.h`).
    fn libc_seal_shrink() -> u32 {
        0x0002
    }
}
