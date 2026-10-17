// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ The display-broker wire protocol, version 2 — byte for byte nvkvm-pv's
//! `src/common/nvkvm_broker_proto.h`, now `broker/common/nvkvm_broker_proto.h` in this repository
//! (the one copy; `tests/proto_mirror.rs` compares every value below with its text, both ways,
//! and the record layouts with the compiled header).
//!
//! Two fixed-size records, little-endian, no length field anywhere: the broker sends 24-byte
//! EVENT packets, the VMM sends 40-byte COMMAND records (one of which carries a descriptor as
//! `SCM_RIGHTS`). A short read builds up to exactly one packet; nothing is ever resynced
//! (`proto.h:25-30`). Encoding here is explicit byte placement at the header's offsets, so no
//! Rust layout question arises: reserved fields are written as zero by construction.

/// `NVKVM_BROKER_PROTO_VERSION`.
pub const PROTO_VERSION: u32 = 2;
/// `NVKVM_BROKER_PKT_SIZE`: one broker → VMM event.
pub const PKT_SIZE: usize = 24;
/// `NVKVM_BROKER_CMD_SIZE`: one VMM → broker command.
pub const CMD_SIZE: usize = 40;

/// `NVKVM_BROKER_EV_HELLO`: `w0` = protocol version, `w1` = capability bits. Always first.
pub const EV_HELLO: u16 = 1;
/// `NVKVM_BROKER_EV_SURFACE`: `x`,`y` = the broker window's size; `w0` = refresh in mHz.
pub const EV_SURFACE: u16 = 2;
/// `NVKVM_BROKER_EV_FRAME`: the display is ready for another frame (pacing).
pub const EV_FRAME: u16 = 3;
/// `NVKVM_BROKER_EV_RELEASE`: `w0`,`w1` = low/high half of the released buffer's id.
pub const EV_RELEASE: u16 = 4;
/// `NVKVM_BROKER_EV_KEY`: `x` = Linux evdev key code, `y` = down.
pub const EV_KEY: u16 = 5;
/// `NVKVM_BROKER_EV_BTN`: `x` = Linux evdev button code, `y` = down.
pub const EV_BTN: u16 = 6;
/// `NVKVM_BROKER_EV_ABS`: `x`,`y` = position, `w0`,`w1` = the range.
pub const EV_ABS: u16 = 7;
/// `NVKVM_BROKER_EV_REL`: `x`,`y` = deltas (only while grabbed).
pub const EV_REL: u16 = 8;
/// `NVKVM_BROKER_EV_WHEEL`: `x` = vertical detents, `y` = horizontal.
pub const EV_WHEEL: u16 = 9;
/// `NVKVM_BROKER_EV_GRAB`: `x` = 1 grab on.
pub const EV_GRAB: u16 = 10;
/// `NVKVM_BROKER_EV_FOCUS`: `x` = 1 active.
pub const EV_FOCUS: u16 = 11;
/// `NVKVM_BROKER_EV_POINTER`: `x` = 1 pointer over the window.
pub const EV_POINTER: u16 = 12;
/// `NVKVM_BROKER_EV_BYE`: `x` = reason.
pub const EV_BYE: u16 = 13;
/// `NVKVM_BROKER_EV_CLOSE`: the user closed the display; `x` = which close.
pub const EV_CLOSE: u16 = 14;
/// `NVKVM_BROKER_EV_CLIPBOARD`: one host clipboard chunk (not used by this relay).
pub const EV_CLIPBOARD: u16 = 15;
/// `NVKVM_BROKER_EV_FORMAT`: the answer to a `QUERY_FORMAT`; `x` = yes, `y` = fourcc,
/// `w0`,`w1` = the modifier.
pub const EV_FORMAT: u16 = 16;

// ── THE CURSOR AND DEVICE SURFACE (2026-10-03; `CAP_CURSOR_STREAM` 2026-10-15) ───────────────
// The broker owns the protocol: these are transcribed from `broker/common/nvkvm_broker_proto.h`,
// not proposed, and `tests/proto_mirror.rs` checks each against its text.

/// ★ `NVKVM_BROKER_EV_DEVICE`: which DRM device the display renders on. Advertised by
/// [`CAP_DEVICE`]; sent once at attach as the packet AFTER the priming FRAME, and again,
/// unsolicited, whenever the display server reports a different device. `x` = `DEVICE_F_*` (0 =
/// the broker does not know), `y` = 0 (reserved), `w0`:`w1` = major:minor of the DRM character
/// device. A hint for choosing a rung; the broker still validates every ATTACH.
pub const EV_DEVICE: u16 = 17;
/// `NVKVM_BROKER_DEVICE_F_KNOWN`: `w0`:`w1` are meaningful.
pub const DEVICE_F_KNOWN: i32 = 1 << 0;
/// `NVKVM_BROKER_DEVICE_F_RENDER`: `w0`:`w1` is a render node (`renderD*`), as reported or as the
/// broker resolved it through sysfs. KNOWN without RENDER = an unresolved (primary) node: compare
/// with care.
pub const DEVICE_F_RENDER: i32 = 1 << 1;
/// `NVKVM_BROKER_CAP_CURSOR`: [`CMD_CURSOR`] is understood; clear ⇒ sending it is a violation and
/// the VMM composes the cursor into the frame.
pub const CAP_CURSOR: u32 = 1 << 10;
/// ★ `NVKVM_BROKER_CAP_CURSOR_STREAM` (HELLO `w1` bit 12): the viewer draws the guest's cursor ITSELF
/// in every input mode, position included (the host pointer in hover, an overlay at the last
/// [`CURSOR_MOVE`] under grab). Then the VMM composes NO cursor into the frame, [`CMD_CURSOR`] also
/// takes `MOVE`, and a SET may carry [`CURSOR_F_XOR`]. Clear ⇒ both are framing violations and the
/// VMM composes the cursor as before. Only with [`CAP_CURSOR`].
pub const CAP_CURSOR_STREAM: u32 = 1 << 12;
/// `NVKVM_BROKER_CURSOR_MOVE`: the cursor image's top-left on the guest frame (signed), in the two
/// hot-spot words, no fd. Only with [`CAP_CURSOR_STREAM`]. In hover the viewer ignores it; under
/// grab it draws the image there.
pub const CURSOR_MOVE: u32 = 4;
/// `NVKVM_BROKER_CURSOR_F_ABS` (the `flags` of a MOVE): `width`/`height` carry the last absolute
/// pointer position the VMM injected, in guest frame pixels. Clear (and both zero) after relative
/// input, or before any was injected.
pub const CURSOR_F_ABS: u16 = 1 << 0;
/// `NVKVM_BROKER_CURSOR_MOVE_LIMIT`: a MOVE beyond ±this is refused by the broker; the VMM clamps
/// far inside it ([`CursorCmd::mov`]).
pub const CURSOR_MOVE_LIMIT: i32 = 1 << 20;
/// `NVKVM_BROKER_CURSOR_F_XOR` (in the command's `flags`, SET only): the image's alpha byte is an
/// inversion MASK (0xff where the guest's XOR cursor inverts the screen, 0 elsewhere; the colour
/// channels carry the XOR colour) and the viewer draws the visible substitute itself.
pub const CURSOR_F_XOR: u16 = 1 << 0;
/// `NVKVM_BROKER_CAP_DEVICE`: an [`EV_DEVICE`] follows the handshake's FRAME (even when its answer
/// is "unknown", so "too old to say" and "could not tell" differ).
pub const CAP_DEVICE: u32 = 1 << 11;
/// `NVKVM_BROKER_CMD_CURSOR`: the guest's pointer image as the host pointer while hovering and not
/// grabbed ([`CursorCmd`]). Only with [`CAP_CURSOR`].
pub const CMD_CURSOR: u16 = 7;
/// `NVKVM_BROKER_CURSOR_SET`: define the image AND show it (exactly one memfd rides along).
pub const CURSOR_SET: u32 = 1;
/// `NVKVM_BROKER_CURSOR_HIDE`: no visible guest cursor (or the VMM composes it). No fd.
pub const CURSOR_HIDE: u32 = 2;
/// `NVKVM_BROKER_CURSOR_SHOW`: show the last SET image again. No fd.
pub const CURSOR_SHOW: u32 = 3;
/// `NVKVM_BROKER_CURSOR_MAX_DIM`: the largest cursor edge.
pub const CURSOR_MAX_DIM: u32 = 256;
/// `NVKVM_BROKER_CURSOR_MAX_STRIDE`: the widest cursor row in bytes (`4 * CURSOR_MAX_DIM`).
pub const CURSOR_MAX_STRIDE: u32 = 4 * CURSOR_MAX_DIM;

// ── PHASE 2: THE BROKER AS TICK SOURCE, LOCK-STEP FRAME REQUESTS (2026-10-17) ────────────────
// `docs/design/V3_DISPLAY_PACING_20261017.md` §11.2; the broker's header is the one copy.

/// ★ `NVKVM_BROKER_CAP_TICK` (HELLO `w1` bit 13): the broker reports the display's refresh as
/// [`EV_TICK`] and runs lock-step frame requests. Only after the VMM opts in ([`CLIENT_TICK`]).
pub const CAP_TICK: u32 = 1 << 13;
/// ★ `NVKVM_BROKER_CLIENT_TICK` (`CMD_CAPS.width` bit 1): the VMM understands [`EV_TICK`] and answers
/// requests. `CMD_CAPS.height` then carries the VMM's vblank ceiling in Hz.
pub const CLIENT_TICK: u32 = 1 << 1;
/// ★ `NVKVM_BROKER_EV_TICK`: one (decimated) host refresh. `x` = `tick_seq` (first tick 1, +1 per
/// emitted tick, wraps skipping 0), `y` = the refresh word ([`TICK_REFRESH_MASK`] bits of refresh in
/// ns, [`TICK_F_WANT_FRAME`], [`TICK_F_CLOCK_ONLY`]), `w0`/`w1` = `present_ns` (CLOCK_MONOTONIC).
pub const EV_TICK: u16 = 18;
/// ★ `NVKVM_BROKER_CMD_NO_CHANGE`: the VMM answers request `width` with "nothing newer".
pub const CMD_NO_CHANGE: u16 = 8;
/// The refresh-period bits of an `EV_TICK` refresh word.
pub const TICK_REFRESH_MASK: u32 = 0x0fff_ffff;
/// `TICK_F_WANT_FRAME` (refresh word bit 28): this tick opens request `tick_seq`.
pub const TICK_F_WANT_FRAME: u32 = 1 << 28;
/// `TICK_F_CLOCK_ONLY` (bit 29): `present_ns` is when the broker LEARNED of the refresh, not the
/// compositor's presentation timestamp.
pub const TICK_F_CLOCK_ONLY: u32 = 1 << 29;
/// Refresh-word bits 30..31: reserved, zero.
pub const TICK_F_RESERVED: u32 = 3 << 30;
/// The shortest `refresh_ns` a tick may carry (1 ms); 0 = unknown is also allowed.
pub const TICK_REFRESH_MIN_NS: u32 = 1_000_000;
/// `NVKVM_BROKER_GUEST_MAX_HZ`: the VMM's vblank ceiling when it says nothing (D3).
pub const GUEST_MAX_HZ: u32 = 75;
/// The lowest ceiling `CMD_CAPS.height` may carry.
pub const GUEST_MIN_HZ: u32 = 24;

/// One `EV_TICK`, decoded and checked (`struct nvkvm_broker_tick_pkt`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TickPkt {
    /// The tick number (never 0).
    pub tick_seq: u32,
    /// The decimated refresh period in ns, 0 = unknown.
    pub refresh_ns: u32,
    /// This tick opens request `tick_seq`.
    pub want: bool,
    /// `present_ns` is the broker's own clock reading, not the compositor's timestamp.
    pub clock_only: bool,
    /// `CLOCK_MONOTONIC`, ns.
    pub present_ns: u64,
}

impl Pkt {
    /// Decode an `EV_TICK`'s operands, or name the framing violation.
    ///
    /// # Errors
    /// `tick_seq` 0, reserved bits, a refresh period outside 1 ms..=268 ms.
    pub fn tick(&self) -> Result<TickPkt, &'static str> {
        let word = self.y.cast_unsigned();
        if word & TICK_F_RESERVED != 0 {
            return Err("EV_TICK with reserved flag bits set");
        }
        let refresh_ns = word & TICK_REFRESH_MASK;
        if refresh_ns != 0 && refresh_ns < TICK_REFRESH_MIN_NS {
            return Err("EV_TICK with a refresh period under 1 ms");
        }
        let tick_seq = self.x.cast_unsigned();
        if tick_seq == 0 {
            return Err("EV_TICK with tick_seq 0");
        }
        Ok(TickPkt {
            tick_seq,
            refresh_ns,
            want: word & TICK_F_WANT_FRAME != 0,
            clock_only: word & TICK_F_CLOCK_ONLY != 0,
            present_ns: self.wide(),
        })
    }

    /// Build an `EV_TICK` (the broker's direction; for tests and fakes).
    #[must_use]
    pub fn tick_pkt(seq: u32, flags: u16, t: &TickPkt) -> Pkt {
        let word = (t.refresh_ns & TICK_REFRESH_MASK)
            | if t.want { TICK_F_WANT_FRAME } else { 0 }
            | if t.clock_only { TICK_F_CLOCK_ONLY } else { 0 };
        Pkt {
            ty: EV_TICK,
            flags,
            seq,
            x: t.tick_seq.cast_signed(),
            y: word.cast_signed(),
            w0: t.present_ns as u32,
            w1: (t.present_ns >> 32) as u32,
        }
    }
}

/// `tick_seq` `a` is newer than `b` (wrapping; 0 never occurs).
#[must_use]
pub fn seq_newer(a: u32, b: u32) -> bool {
    a != b && a.wrapping_sub(b) < 0x8000_0000
}

/// `NVKVM_BROKER_CLOSE_POWERDOWN`.
pub const CLOSE_POWERDOWN: i32 = 0;
/// `NVKVM_BROKER_CLOSE_FORCE`.
pub const CLOSE_FORCE: i32 = 1;

/// `NVKVM_BROKER_CAP_KEYBOARD`.
pub const CAP_KEYBOARD: u32 = 1 << 0;
/// `NVKVM_BROKER_CAP_ABS_POINTER`.
pub const CAP_ABS_POINTER: u32 = 1 << 1;
/// `NVKVM_BROKER_CAP_REL_POINTER`.
pub const CAP_REL_POINTER: u32 = 1 << 2;
/// `NVKVM_BROKER_CAP_POINTER_LOCK`.
pub const CAP_POINTER_LOCK: u32 = 1 << 3;
/// `NVKVM_BROKER_CAP_TOTAL_GRAB`.
pub const CAP_TOTAL_GRAB: u32 = 1 << 4;
/// `NVKVM_BROKER_CAP_FOCUS_EVENTS`.
pub const CAP_FOCUS_EVENTS: u32 = 1 << 5;
/// `NVKVM_BROKER_CAP_FULLSCREEN`.
pub const CAP_FULLSCREEN: u32 = 1 << 6;
/// `NVKVM_BROKER_CAP_DMABUF`: the session accepts dma-bufs (required by this relay).
pub const CAP_DMABUF: u32 = 1 << 7;
/// `NVKVM_BROKER_CAP_MODIFIERS`: explicit modifiers are negotiated; clear ⇒ only
/// `DRM_FORMAT_MOD_INVALID` is accepted.
pub const CAP_MODIFIERS: u32 = 1 << 8;
/// `NVKVM_BROKER_CAP_RELEASE`: `EV_RELEASE` is real, not synthesised.
pub const CAP_RELEASE: u32 = 1 << 9;

/// `NVKVM_BROKER_BYE_SHUTDOWN`.
pub const BYE_SHUTDOWN: i32 = 0;
/// `NVKVM_BROKER_BYE_DISPLAY_LOST`.
pub const BYE_DISPLAY_LOST: i32 = 1;
/// `NVKVM_BROKER_BYE_PROTOCOL`.
pub const BYE_PROTOCOL: i32 = 2;

/// `NVKVM_BROKER_F_GRABBED` (mirrored on every packet).
pub const F_GRABBED: u16 = 1 << 0;
/// `NVKVM_BROKER_F_FOCUSED`.
pub const F_FOCUSED: u16 = 1 << 1;
/// `NVKVM_BROKER_F_FULLSCREEN`.
pub const F_FULLSCREEN: u16 = 1 << 2;

/// `NVKVM_BROKER_CMD_ATTACH`: exactly one descriptor rides with it.
pub const CMD_ATTACH: u16 = 1;
/// `NVKVM_BROKER_CMD_COMMIT`: present the last ATTACHed buffer; all fields zero.
pub const CMD_COMMIT: u16 = 2;
/// `NVKVM_BROKER_CMD_WINDOW`: ask for a window of `width` x `height`.
pub const CMD_WINDOW: u16 = 3;
/// `NVKVM_BROKER_CMD_CLIPBOARD` (not used by this relay).
pub const CMD_CLIPBOARD: u16 = 4;
/// `NVKVM_BROKER_CMD_CAPS`: `width` = `CLIENT_*` bits; everything else zero.
pub const CMD_CAPS: u16 = 5;
/// `NVKVM_BROKER_CMD_QUERY_FORMAT`: `fourcc` and `modifier` are the question.
pub const CMD_QUERY_FORMAT: u16 = 6;

/// `NVKVM_BROKER_CMD_F_SHM`: the ATTACHed descriptor is a sealed memfd to present as shared
/// memory, not a dma-buf.
pub const CMD_F_SHM: u16 = 1 << 0;
/// `NVKVM_BROKER_CLIENT_CLIPBOARD` (`CMD_CAPS` `width` bit). Never set by this relay.
pub const CLIENT_CLIPBOARD: u32 = 1 << 0;
/// `NVKVM_BROKER_MAX_DIM`: the largest edge the broker accepts.
pub const MAX_DIM: u32 = 8192;

/// `DRM_FORMAT_XRGB8888` (`'X' 'R' '2' '4'`): the console's `x8r8g8b8` byte order.
pub const FOURCC_XR24: u32 = 0x3432_5258;
/// `DRM_FORMAT_ARGB8888`.
pub const FOURCC_AR24: u32 = 0x3432_5241;
/// `DRM_FORMAT_MOD_LINEAR`.
pub const MOD_LINEAR: u64 = 0;
/// `DRM_FORMAT_MOD_INVALID`: the implicit (driver-chosen) layout.
pub const MOD_INVALID: u64 = 0x00ff_ffff_ffff_ffff;

/// One broker → VMM event, decoded (`struct nvkvm_broker_pkt`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pkt {
    /// `EV_*`.
    pub ty: u16,
    /// `F_*` (mirrored state).
    pub flags: u16,
    /// The broker's per-connection counter.
    pub seq: u32,
    /// First signed operand.
    pub x: i32,
    /// Second signed operand.
    pub y: i32,
    /// First unsigned operand.
    pub w0: u32,
    /// Second unsigned operand.
    pub w1: u32,
}

impl Pkt {
    /// Decode exactly one packet (`type@0 flags@2 seq@4 x@8 y@12 w0@16 w1@20`).
    #[must_use]
    pub fn decode(b: &[u8; PKT_SIZE]) -> Pkt {
        let u16_at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
        let u32_at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        Pkt {
            ty: u16_at(0),
            flags: u16_at(2),
            seq: u32_at(4),
            x: u32_at(8) as i32,
            y: u32_at(12) as i32,
            w0: u32_at(16),
            w1: u32_at(20),
        }
    }

    /// Encode (the broker's direction; for tests and fakes).
    #[must_use]
    pub fn encode(&self) -> [u8; PKT_SIZE] {
        let mut b = [0u8; PKT_SIZE];
        b[0..2].copy_from_slice(&self.ty.to_le_bytes());
        b[2..4].copy_from_slice(&self.flags.to_le_bytes());
        b[4..8].copy_from_slice(&self.seq.to_le_bytes());
        b[8..12].copy_from_slice(&self.x.to_le_bytes());
        b[12..16].copy_from_slice(&self.y.to_le_bytes());
        b[16..20].copy_from_slice(&self.w0.to_le_bytes());
        b[20..24].copy_from_slice(&self.w1.to_le_bytes());
        b
    }

    /// `w0 | w1 << 32` — a buffer id (`RELEASE`) or a modifier (`FORMAT`).
    #[must_use]
    pub fn wide(&self) -> u64 {
        u64::from(self.w0) | (u64::from(self.w1) << 32)
    }
}

/// One VMM → broker command (`struct nvkvm_broker_cmd`); `reserved1` is always zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cmd {
    /// `CMD_*`.
    pub ty: u16,
    /// `CMD_F_*`.
    pub flags: u16,
    /// Width (or `CLIENT_*` bits for `CAPS`).
    pub width: u32,
    /// Height.
    pub height: u32,
    /// Bytes per row of plane 0.
    pub stride: u32,
    /// Byte offset of plane 0.
    pub offset: u32,
    /// `DRM_FORMAT_*`.
    pub fourcc: u32,
    /// `DRM_FORMAT_MOD_*`.
    pub modifier: u64,
    /// Advisory client counter.
    pub seq: u32,
}

impl Cmd {
    /// Encode at the header's offsets (`type@0 flags@2 width@4 height@8 stride@12 offset@16
    /// fourcc@20 modifier@24 seq@32 reserved1@36`).
    #[must_use]
    pub fn encode(&self) -> [u8; CMD_SIZE] {
        let mut b = [0u8; CMD_SIZE];
        b[0..2].copy_from_slice(&self.ty.to_le_bytes());
        b[2..4].copy_from_slice(&self.flags.to_le_bytes());
        b[4..8].copy_from_slice(&self.width.to_le_bytes());
        b[8..12].copy_from_slice(&self.height.to_le_bytes());
        b[12..16].copy_from_slice(&self.stride.to_le_bytes());
        b[16..20].copy_from_slice(&self.offset.to_le_bytes());
        b[20..24].copy_from_slice(&self.fourcc.to_le_bytes());
        b[24..32].copy_from_slice(&self.modifier.to_le_bytes());
        b[32..36].copy_from_slice(&self.seq.to_le_bytes());
        b
    }

    /// Decode (the broker's direction; for tests and fakes). `None` when `reserved1` is not 0.
    #[must_use]
    pub fn decode(b: &[u8; CMD_SIZE]) -> Option<Cmd> {
        let u16_at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
        let u32_at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        if u32_at(36) != 0 {
            return None;
        }
        let mut m = [0u8; 8];
        m.copy_from_slice(&b[24..32]);
        Some(Cmd {
            ty: u16_at(0),
            flags: u16_at(2),
            width: u32_at(4),
            height: u32_at(8),
            stride: u32_at(12),
            offset: u32_at(16),
            fourcc: u32_at(20),
            modifier: u64::from_le_bytes(m),
            seq: u32_at(32),
        })
    }

    /// `QUERY_FORMAT(fourcc, modifier)`.
    #[must_use]
    pub fn query(fourcc: u32, modifier: u64) -> Cmd {
        Cmd {
            ty: CMD_QUERY_FORMAT,
            fourcc,
            modifier,
            ..Cmd::default()
        }
    }

    /// `WINDOW(w, h)`.
    #[must_use]
    pub fn window(width: u32, height: u32) -> Cmd {
        Cmd {
            ty: CMD_WINDOW,
            width,
            height,
            ..Cmd::default()
        }
    }

    /// `COMMIT` — every descriptor field zero.
    #[must_use]
    pub fn commit() -> Cmd {
        Cmd {
            ty: CMD_COMMIT,
            ..Cmd::default()
        }
    }

    /// `CAPS(bits)`.
    #[must_use]
    pub fn caps(bits: u32) -> Cmd {
        Cmd {
            ty: CMD_CAPS,
            width: bits,
            ..Cmd::default()
        }
    }

    /// ★ `CAPS` opting in to ticks: [`CLIENT_TICK`] and the vblank ceiling in Hz (`height`; clamped
    /// into [`GUEST_MIN_HZ`]`..=`[`GUEST_MAX_HZ`], 0 = [`GUEST_MAX_HZ`]).
    #[must_use]
    pub fn caps_tick(max_hz: u32) -> Cmd {
        Cmd {
            ty: CMD_CAPS,
            width: CLIENT_TICK,
            height: if max_hz == 0 {
                GUEST_MAX_HZ
            } else {
                max_hz.clamp(GUEST_MIN_HZ, GUEST_MAX_HZ)
            },
            ..Cmd::default()
        }
    }

    /// ★ `COMMIT` answering request `req` (tick-negotiated connections): `seq` = `req`.
    #[must_use]
    pub fn commit_for(req: u32) -> Cmd {
        Cmd {
            ty: CMD_COMMIT,
            seq: req,
            ..Cmd::default()
        }
    }

    /// ★ `NO_CHANGE(req)`: nothing newer to show for request `req`.
    #[must_use]
    pub fn no_change(req: u32) -> Cmd {
        Cmd {
            ty: CMD_NO_CHANGE,
            width: req,
            ..Cmd::default()
        }
    }
}

/// ★ `struct nvkvm_broker_cursor_cmd` — the 40-byte command record with ATTACH's buffer
/// description at the same offsets (`type@0 flags@2 width@4 height@8 stride@12 offset@16
/// fourcc@20`), the hot spot where ATTACH keeps its modifier (`hot_x@24 hot_y@28`), `op@32` where
/// ATTACH keeps its seq, `reserved1@36` = 0. `flags` is 0, or [`CURSOR_F_XOR`] on a SET to a broker
/// that advertised [`CAP_CURSOR_STREAM`]. Pixels are premultiplied `DRM_FORMAT_ARGB8888` only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CursorCmd {
    /// `flags@2`: 0, or [`CURSOR_F_XOR`] (SET only).
    pub flags: u16,
    /// Pixels per row (SET: `1..=CURSOR_MAX_DIM`).
    pub width: u32,
    /// Rows (SET: `1..=CURSOR_MAX_DIM`).
    pub height: u32,
    /// Bytes per row (SET: `4 * width ..= CURSOR_MAX_STRIDE`).
    pub stride: u32,
    /// Byte offset of row 0 in the memfd.
    pub offset: u32,
    /// [`FOURCC_AR24`] on SET.
    pub fourcc: u32,
    /// Hot spot column (`< width`).
    pub hot_x: u32,
    /// Hot spot row (`< height`).
    pub hot_y: u32,
    /// [`CURSOR_SET`], [`CURSOR_HIDE`] or [`CURSOR_SHOW`].
    pub op: u32,
}

impl CursorCmd {
    /// ★ A SET of a `width`x`height` image at `stride` bytes a row from `offset` in a memfd of
    /// `memfd_bytes` bytes — `None` unless every bound the broker checks holds (computed in 64
    /// bits), so a refused SET is never sent.
    #[must_use]
    pub fn set(
        width: u32,
        height: u32,
        stride: u32,
        offset: u32,
        hot: (u32, u32),
        memfd_bytes: u64,
    ) -> Option<CursorCmd> {
        let ok = (1..=CURSOR_MAX_DIM).contains(&width)
            && (1..=CURSOR_MAX_DIM).contains(&height)
            && (u64::from(width) * 4..=u64::from(CURSOR_MAX_STRIDE)).contains(&u64::from(stride))
            && hot.0 < width
            && hot.1 < height
            && u64::from(offset) + u64::from(stride) * u64::from(height - 1) + u64::from(width) * 4
                <= memfd_bytes;
        ok.then_some(CursorCmd {
            flags: 0,
            width,
            height,
            stride,
            offset,
            fourcc: FOURCC_AR24,
            hot_x: hot.0,
            hot_y: hot.1,
            op: CURSOR_SET,
        })
    }

    /// ★ The same SET, flagged [`CURSOR_F_XOR`]: the image is an XOR cursor's inversion mask.
    #[must_use]
    pub fn xor(self) -> CursorCmd {
        CursorCmd {
            flags: CURSOR_F_XOR,
            ..self
        }
    }

    /// ★ MOVE, ONE atomic pair: the image's top-left at `(x, y)` on the guest frame (the hot-spot
    /// words) and `abs`, the last absolute pointer position the VMM injected in the same frame
    /// pixels (`width`/`height`, with [`CURSOR_F_ABS`]; `None`: the last input was relative, or
    /// none was injected yet — flag clear, both zero). Every value is clamped to `-CURSOR_MAX_DIM
    /// ..= MAX_DIM` (outside of which a cursor cannot show) as signed 32-bit; every other field but
    /// `op` zero. Only for a broker that advertised [`CAP_CURSOR_STREAM`].
    #[must_use]
    pub fn mov(x: i32, y: i32, abs: Option<(i32, i32)>) -> CursorCmd {
        let c = |v: i32| v.clamp(-(CURSOR_MAX_DIM as i32), MAX_DIM as i32).cast_unsigned();
        let (ax, ay) = abs.unwrap_or((0, 0));
        CursorCmd {
            flags: if abs.is_some() { CURSOR_F_ABS } else { 0 },
            width: if abs.is_some() { c(ax) } else { 0 },
            height: if abs.is_some() { c(ay) } else { 0 },
            hot_x: c(x),
            hot_y: c(y),
            op: CURSOR_MOVE,
            ..CursorCmd::default()
        }
    }

    /// HIDE: every field but `op` zero.
    #[must_use]
    pub fn hide() -> CursorCmd {
        CursorCmd {
            op: CURSOR_HIDE,
            ..CursorCmd::default()
        }
    }

    /// SHOW: every field but `op` zero.
    #[must_use]
    pub fn show() -> CursorCmd {
        CursorCmd {
            op: CURSOR_SHOW,
            ..CursorCmd::default()
        }
    }

    /// Encode at the header's offsets; `reserved1` is zero by construction.
    #[must_use]
    pub fn encode(&self) -> [u8; CMD_SIZE] {
        let mut b = [0u8; CMD_SIZE];
        b[0..2].copy_from_slice(&CMD_CURSOR.to_le_bytes());
        b[2..4].copy_from_slice(&self.flags.to_le_bytes());
        for (at, v) in [
            (4, self.width),
            (8, self.height),
            (12, self.stride),
            (16, self.offset),
            (20, self.fourcc),
            (24, self.hot_x),
            (28, self.hot_y),
            (32, self.op),
        ] {
            b[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        b
    }
}

/// A fourcc as its four characters, for log lines.
#[must_use]
pub fn fourcc_name(f: u32) -> String {
    f.to_le_bytes()
        .iter()
        .map(|&c| {
            if c.is_ascii_graphic() {
                char::from(c)
            } else {
                '?'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Field offsets, checked by planting a distinct value in each field and finding it at the
    /// header's offset — a swapped pair of `u32` would decode, so only placement tests it.
    #[test]
    fn every_command_field_lands_at_the_header_offset() {
        let c = Cmd {
            ty: 0x0102,
            flags: 0x0304,
            width: 0x1111_1111,
            height: 0x2222_2222,
            stride: 0x3333_3333,
            offset: 0x4444_4444,
            fourcc: 0x5555_5555,
            modifier: 0x6666_6666_7777_7777,
            seq: 0x8888_8888,
        };
        let b = c.encode();
        assert_eq!(&b[0..2], &[0x02, 0x01]);
        assert_eq!(&b[2..4], &[0x04, 0x03]);
        for (off, v) in [
            (4, 0x11u8),
            (8, 0x22),
            (12, 0x33),
            (16, 0x44),
            (20, 0x55),
            (24, 0x77),
            (28, 0x66),
            (32, 0x88),
        ] {
            assert_eq!(b[off..off + 4], [v; 4], "offset {off}");
        }
        assert_eq!(&b[36..40], &[0; 4], "reserved1 is zero by construction");
        assert_eq!(Cmd::decode(&b), Some(c));
        let mut bad = b;
        bad[39] = 1;
        assert_eq!(Cmd::decode(&bad), None, "a non-zero reserved1 is refused");
    }

    #[test]
    fn every_packet_field_lands_at_the_header_offset() {
        let p = Pkt {
            ty: EV_ABS,
            flags: F_FOCUSED,
            seq: 0x0a0b_0c0d,
            x: -2,
            y: 0x1234,
            w0: 1920,
            w1: 1080,
        };
        let b = p.encode();
        assert_eq!(&b[4..8], &[0x0d, 0x0c, 0x0b, 0x0a]);
        assert_eq!(&b[8..12], &[0xfe, 0xff, 0xff, 0xff], "x is signed");
        assert_eq!(Pkt::decode(&b), p);
        let r = Pkt {
            ty: EV_RELEASE,
            w0: 0xdead_beef,
            w1: 0x1,
            ..Pkt::default()
        };
        assert_eq!(r.wide(), 0x1_dead_beef);
    }

    /// ★ The cursor record at the broker's offsets, and SET refused unless every bound the broker
    /// checks holds (each bound one past its edge).
    #[test]
    fn the_cursor_record_and_its_bounds_are_the_brokers() {
        let c = CursorCmd::set(64, 32, 1024, 16, (3, 31), 16 + 1024 * 31 + 256).unwrap();
        let b = c.encode();
        assert_eq!(&b[0..2], &CMD_CURSOR.to_le_bytes());
        assert_eq!(&b[2..4], &[0, 0], "flags");
        for (at, v) in [
            (4, 64u32),
            (8, 32),
            (12, 1024),
            (16, 16),
            (20, FOURCC_AR24),
            (24, 3),
            (28, 31),
            (32, CURSOR_SET),
        ] {
            assert_eq!(&b[at..at + 4], &v.to_le_bytes(), "+{at}");
        }
        assert_eq!(&b[36..40], &[0; 4], "reserved1");
        let fits = 16 + 1024 * 31 + 256;
        for bad in [
            CursorCmd::set(0, 32, 1024, 16, (0, 0), fits),
            CursorCmd::set(257, 1, 1028, 0, (0, 0), 1 << 20),
            CursorCmd::set(64, 32, 255, 16, (0, 0), fits),
            CursorCmd::set(64, 32, 1025, 16, (0, 0), 1 << 20),
            CursorCmd::set(64, 32, 1024, 16, (64, 0), fits),
            CursorCmd::set(64, 32, 1024, 16, (0, 32), fits),
            CursorCmd::set(64, 32, 1024, 16, (0, 0), fits - 1),
            CursorCmd::set(256, 256, 1024, u32::MAX, (0, 0), 1 << 32),
        ] {
            assert_eq!(bad, None);
        }
        assert!(CursorCmd::set(256, 256, 1024, 0, (255, 255), 256 * 1024).is_some());
        for (c, op) in [
            (CursorCmd::hide(), CURSOR_HIDE),
            (CursorCmd::show(), CURSOR_SHOW),
        ] {
            let b = c.encode();
            assert_eq!(&b[32..36], &op.to_le_bytes());
            assert!(b[2..32].iter().chain(&b[36..]).all(|x| *x == 0));
        }
    }

    #[test]
    fn the_helpers_zero_everything_they_do_not_name() {
        assert_eq!(Cmd::commit().encode()[2..], [0u8; CMD_SIZE - 2]);
        let q = Cmd::query(FOURCC_XR24, MOD_INVALID).encode();
        assert_eq!(&q[4..20], &[0u8; 16]);
        assert_eq!(fourcc_name(FOURCC_XR24), "XR24");
    }

    /// ★ The `EV_TICK` overlay at the header's offsets, and every framing rule of §11.2.
    #[test]
    fn the_tick_event_lands_at_the_header_offsets_and_its_violations_are_named() {
        let t = TickPkt {
            tick_seq: 0xa1b2_c3d4,
            refresh_ns: 8_333_333,
            want: true,
            clock_only: false,
            present_ns: 0x0123_4567_89ab_cdef,
        };
        let b = Pkt::tick_pkt(9, F_FOCUSED, &t).encode();
        assert_eq!(&b[0..2], &EV_TICK.to_le_bytes());
        assert_eq!(&b[2..4], &F_FOCUSED.to_le_bytes(), "the mirrored flags stay the mirrored flags");
        assert_eq!(&b[8..12], &0xa1b2_c3d4u32.to_le_bytes(), "tick_seq@8");
        assert_eq!(
            &b[12..16],
            &(8_333_333u32 | TICK_F_WANT_FRAME).to_le_bytes(),
            "refresh word@12"
        );
        assert_eq!(&b[16..24], &0x0123_4567_89ab_cdefu64.to_le_bytes(), "present_ns@16");
        assert_eq!(Pkt::decode(&b).tick(), Ok(t));
        let base = Pkt::tick_pkt(1, 0, &t);
        for (name, bad) in [
            ("seq 0", Pkt { x: 0, ..base }),
            ("reserved 30", Pkt { y: (base.y.cast_unsigned() | 1 << 30).cast_signed(), ..base }),
            ("reserved 31", Pkt { y: (base.y.cast_unsigned() | 1 << 31).cast_signed(), ..base }),
            (
                "refresh 999999 ns",
                Pkt { y: 999_999, ..base },
            ),
        ] {
            assert!(bad.tick().is_err(), "{name}");
        }
        assert!(Pkt { y: 0, ..base }.tick().is_ok(), "refresh 0 = unknown");
        assert!(
            Pkt { y: TICK_REFRESH_MASK.cast_signed(), ..base }.tick().is_ok(),
            "the largest refresh"
        );
        // the commands
        assert_eq!(Cmd::commit_for(7).encode()[32..36], 7u32.to_le_bytes());
        let n = Cmd::no_change(7).encode();
        assert_eq!(&n[0..2], &CMD_NO_CHANGE.to_le_bytes());
        assert_eq!(&n[4..8], &7u32.to_le_bytes());
        assert!(n[8..].iter().all(|x| *x == 0));
        let c = Cmd::caps_tick(0).encode();
        assert_eq!(&c[4..8], &CLIENT_TICK.to_le_bytes());
        assert_eq!(&c[8..12], &75u32.to_le_bytes());
        assert_eq!(Cmd::caps_tick(10).height, 24);
        assert_eq!(Cmd::caps_tick(60).height, 60);
        assert_eq!(Cmd::caps_tick(900).height, 75);
    }

    /// Wrapping order of tick numbers, around the wrap and never equal.
    #[test]
    fn tick_numbers_order_around_the_wrap() {
        assert!(seq_newer(2, 1));
        assert!(!seq_newer(1, 2));
        assert!(!seq_newer(5, 5));
        assert!(seq_newer(1, u32::MAX), "1 follows the wrap (0 is skipped)");
        assert!(!seq_newer(u32::MAX, 1));
        assert!(seq_newer(0x8000_0000, 1), "half the range ahead is still newer");
        assert!(!seq_newer(0x8000_0001, 1), "more than half is stale");
    }
}
