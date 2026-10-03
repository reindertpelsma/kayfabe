// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★★ **`kf-broker` — the display-broker relay, VMM-agnostic** (`docs/design/V3_DISPLAY.md` §8).
//!
//! nvkvm-pv's display broker (`nvkvm-pv src/broker/`, pinned at `368d2db`, installed
//! separately) is a separate process that owns the host window, the compositor connection and
//! the input grab. The VMM holds one unix socket: frames cross as descriptors (`ATTACH` +
//! `SCM_RIGHTS`) and come back as `RELEASE`; input, pacing and window state come back as fixed
//! 24-byte packets. This crate is the VMM half, with the wire protocol unchanged:
//!
//! - [`wire`] — protocol v2, byte-compatible with `proto/nvkvm_broker_proto.h` (vendored
//!   verbatim; `tests/proto_mirror.rs` compares the two);
//! - [`slots`] — the [`FrameRing`]: the display's copy targets, shared by the display worker,
//!   the VMM's console and this relay through ONE atomic word;
//! - [`conn`] — the [`Relay`]: connect, peer check, HELLO, replay, the owed frame, pacing,
//!   RELEASE accounting, reclaim, reconnect with backoff — everything decided here, nothing
//!   blocking, deterministic under a caller-supplied clock;
//! - [`link`] — the real socket ([`UnixLink`]) and the peer policy's inputs (root, the VMM's
//!   effective uid at each connect, `display-broker-uid`);
//! - [`cursor`] — ★ the guest's cursor as the host pointer in hover mode (`OWNER_RULINGS.md` §O):
//!   the [`CursorShare`] the worker and the relay meet in, and what the broker is sent.
//!
//! The VMM's part is small and is the only VMM-specific code: register the socket and a timer
//! ([`Host`]), wake the relay when the worker publishes a frame, and inject [`Input`] through
//! its own input devices (for QEMU, `qemu/hw/misc/kf3/kf3.c`).
//!
//! Attribution: protocol semantics and the input mapping are ported from nvkvm-pv
//! `src/qemu/nvkvm_display_relay.c` at `368d2db` (Apache-2.0, same author); the vendored header
//! keeps its own SPDX line (`GPL-2.0 OR Apache-2.0`).

pub mod conn;
pub mod cursor;
pub mod gpucopy;
pub mod link;
pub mod slots;
pub mod wire;

pub use conn::{Counters, Host, Input, Link, Recv, Relay, RelayConfig, Rung, Sent};
#[doc(hidden)]
pub use conn::{LogCapture, capture_log};
pub use cursor::{CursorImage, CursorMode, CursorShare, CursorWant};
pub use link::{MAX_BROKER_UID, UnixLink, broker_uid_property, broker_uids, effective_uid};
pub use slots::{
    FrameGeom, FrameRing, InstallRefusal, Kind, SlotFds, Take, VramFds, VramGeom, frame_bytes,
};
