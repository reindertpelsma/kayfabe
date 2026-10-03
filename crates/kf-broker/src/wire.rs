// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ The display-broker wire protocol, version 2 — byte for byte nvkvm-pv's
//! `src/common/nvkvm_broker_proto.h` at `368d2db` (vendored verbatim as
//! `proto/nvkvm_broker_proto.h`; `tests/proto_mirror.rs` compares every value below with its
//! text).
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

/// ★ **AHEAD OF THE VENDORED HEADER** (2026-10-03, `docs/design/V3_DISPLAY.md` §8.11) — the
/// compositor's DRM device: `x` = major, `y` = minor of the node it renders on (primary or render
/// node), `x < 0` = the broker cannot tell. Append-only in protocol v2, sent once after HELLO by a
/// broker that knows it; an older broker never sends it (unknown types are skipped exactly, so the
/// relay then falls back to the acknowledgement detector). The value is THIS relay's proposal to
/// nvkvm-pv's broker (the coordinator's default for the owner: the change goes into nvkvm-pv's
/// broker in the same revision as the cursor message); `tests/proto_mirror.rs` asserts the
/// vendored header does not define it yet, so the day it does, the test forces the value to be
/// checked against the header's.
pub const EV_DEVICE: u16 = 17;

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

    #[test]
    fn the_helpers_zero_everything_they_do_not_name() {
        assert_eq!(Cmd::commit().encode()[2..], [0u8; CMD_SIZE - 2]);
        let q = Cmd::query(FOURCC_XR24, MOD_INVALID).encode();
        assert_eq!(&q[4..20], &[0u8; 16]);
        assert_eq!(fourcc_name(FOURCC_XR24), "XR24");
    }
}
