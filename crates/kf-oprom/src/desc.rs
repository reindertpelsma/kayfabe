// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! The `KFGP` descriptor: what the GOP firmware needs to know about the framebuffer kf3 prepared.
//!
//! It sits in the ROM's first image, after the PCI data structure and before the PE, at
//! `(PcirOffset + PCIR length + 0xF) & !0xF` — the 16-byte-aligned slot after PCIR that NVIDIA's own
//! VBIOS parser also expects a following structure at (`ogkm-580:
//! src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_vbios_tu102.c:331`). It is VMM-authored and
//! the ROM BAR is read-only, so a guest cannot change it; the firmware still decodes it strictly,
//! because a firmware that guesses is a firmware that draws past the end of a BAR.
//!
//! ## Layout, version 1 (little-endian)
//!
//! | offset | size | field |
//! |---|---|---|
//! | 0x00 | 4 | magic `KFGP` |
//! | 0x04 | 2 | version = 1 |
//! | 0x06 | 2 | length of the whole descriptor, checksum byte included |
//! | 0x08 | 2 | vendor id (an echo of PCIR's; the firmware also checks it against config space) |
//! | 0x0A | 2 | device id (likewise) |
//! | 0x0C | 1 | framebuffer BAR index (kf3: 1) |
//! | 0x0D | 1 | pixel format: 1 = XRGB8888 (bytes B, G, R, X — UEFI `PixelBlueGreenRedReserved8BitPerColor`) |
//! | 0x0E | 2 | flags: none defined at version 1, must be 0 |
//! | 0x10 | 8 | framebuffer offset inside the BAR (kf3: 0) |
//! | 0x18 | 8 | framebuffer size G in bytes |
//! | 0x20 | 4 | width |
//! | 0x24 | 4 | height |
//! | 0x28 | 4 | pitch in bytes |
//! | 0x2C | 2 | EDID length (0..=256) |
//! | 0x2E | 2 | reserved, 0 |
//! | 0x30 | n | EDID |
//! | 0x30+n | 1 | checksum: all `length` bytes sum to 0 mod 256 |
//!
//! ## Geometry, and why these roundings
//! - `pitch = align_up(4·W, 256)`: NVKMS rounds a console's pitch up to 256 bytes (`ogkm-580:
//!   src/nvidia-modeset/src/nvkms-rm.c:4880-4884`), so the firmware's console is importable as-is.
//! - `G = align_up(pitch·H, 64 KiB)`: CPU-RM reserves `NV_ALIGN_UP(fbConsoleSize, 64 KiB)` for a
//!   preserved console (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/osinit.c:1092`).
//! - At 1920x1080 that is pitch 7680 and G = 0x7F0000.

use core::fmt;

use crate::rom::{self, CODE_TYPE_EFI, ParseError, le16, le32, le64};

/// The descriptor's magic.
pub const MAGIC: [u8; 4] = *b"KFGP";
/// The descriptor version this crate encodes and decodes.
pub const VERSION: u16 = 1;
/// Bytes before the EDID.
pub const HEADER_LEN: usize = 0x30;
/// The longest EDID a descriptor carries: a base block and one extension.
pub const EDID_MAX: usize = 256;
/// The longest descriptor.
pub const MAX_LEN: usize = HEADER_LEN + EDID_MAX + 1;
/// Pixel format 1: XRGB8888.
pub const FORMAT_XRGB8888: u8 = 1;
/// Bytes per pixel of [`FORMAT_XRGB8888`].
pub const BYTES_PER_PIXEL: u32 = 4;
/// Pitch alignment (bytes).
pub const PITCH_ALIGN: u32 = 256;
/// Framebuffer size alignment (bytes).
pub const FB_ALIGN: u64 = 64 * 1024;
/// The largest width or height a descriptor may state.
pub const MAX_DIM: u32 = 16384;
/// The highest PCI BAR index.
pub const MAX_BAR: u8 = 5;

/// One mode's framebuffer geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    /// Visible width in pixels.
    pub width: u32,
    /// Visible height in lines.
    pub height: u32,
    /// Bytes from one line to the next.
    pub pitch: u32,
    /// Bytes of BAR the framebuffer owns (G): at least `pitch · height`.
    pub fb_size: u64,
}

/// Why a geometry is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryError {
    /// Width or height is 0.
    ZeroSize,
    /// Width or height exceeds [`MAX_DIM`].
    TooLarge {
        /// Width asked for.
        width: u32,
        /// Height asked for.
        height: u32,
    },
    /// The pitch is not a multiple of [`PITCH_ALIGN`].
    PitchUnaligned {
        /// The pitch.
        pitch: u32,
    },
    /// The pitch is shorter than one line of pixels.
    PitchTooSmall {
        /// The pitch.
        pitch: u32,
        /// `4 · width`.
        min: u64,
    },
    /// The framebuffer is smaller than `pitch · height`.
    FramebufferTooSmall {
        /// The size stated.
        fb_size: u64,
        /// `pitch · height`.
        min: u64,
    },
}

impl fmt::Display for GeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            GeometryError::ZeroSize => write!(f, "width or height is 0"),
            GeometryError::TooLarge { width, height } => {
                write!(f, "{width}x{height} exceeds {MAX_DIM} in a dimension")
            }
            GeometryError::PitchUnaligned { pitch } => {
                write!(f, "pitch {pitch} is not a multiple of {PITCH_ALIGN}")
            }
            GeometryError::PitchTooSmall { pitch, min } => {
                write!(f, "pitch {pitch} is shorter than a line ({min} bytes)")
            }
            GeometryError::FramebufferTooSmall { fb_size, min } => {
                write!(
                    f,
                    "framebuffer {fb_size:#x} is smaller than pitch*height {min:#x}"
                )
            }
        }
    }
}

const fn align_up(v: u64, a: u64) -> u64 {
    v.div_ceil(a) * a
}

impl Geometry {
    /// The authored geometry for a `width`×`height` XRGB8888 mode: pitch rounded up to 256 bytes,
    /// framebuffer rounded up to 64 KiB (module docs).
    pub fn for_mode(width: u32, height: u32) -> Result<Geometry, GeometryError> {
        check_dims(width, height)?;
        // ≤ 16384 · 4 rounded to 256: fits u32.
        let pitch = align_up(
            u64::from(width) * u64::from(BYTES_PER_PIXEL),
            u64::from(PITCH_ALIGN),
        ) as u32;
        let fb_size = align_up(u64::from(pitch) * u64::from(height), FB_ALIGN);
        let g = Geometry {
            width,
            height,
            pitch,
            fb_size,
        };
        g.check()?;
        Ok(g)
    }

    /// Bytes the visible lines span: `pitch · height`.
    #[must_use]
    pub fn min_fb_size(&self) -> u64 {
        u64::from(self.pitch) * u64::from(self.height)
    }

    /// Check the invariants every consumer relies on.
    pub fn check(&self) -> Result<(), GeometryError> {
        check_dims(self.width, self.height)?;
        if !self.pitch.is_multiple_of(PITCH_ALIGN) {
            return Err(GeometryError::PitchUnaligned { pitch: self.pitch });
        }
        let line = u64::from(self.width) * u64::from(BYTES_PER_PIXEL);
        if u64::from(self.pitch) < line {
            return Err(GeometryError::PitchTooSmall {
                pitch: self.pitch,
                min: line,
            });
        }
        if self.fb_size < self.min_fb_size() {
            return Err(GeometryError::FramebufferTooSmall {
                fb_size: self.fb_size,
                min: self.min_fb_size(),
            });
        }
        Ok(())
    }
}

fn check_dims(width: u32, height: u32) -> Result<(), GeometryError> {
    if width == 0 || height == 0 {
        return Err(GeometryError::ZeroSize);
    }
    if width > MAX_DIM || height > MAX_DIM {
        return Err(GeometryError::TooLarge { width, height });
    }
    Ok(())
}

/// Where the boot framebuffer lives, and its geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootFramebuffer {
    /// PCI BAR index (kf3: 1).
    pub bar: u8,
    /// Offset of the framebuffer inside the BAR (kf3: 0).
    pub offset: u64,
    /// The one mode.
    pub geometry: Geometry,
}

impl BootFramebuffer {
    /// Whether `[offset, offset + G)` lies inside a BAR of `bar_len` bytes. The firmware asks this of
    /// the BAR the PCI bus driver actually assigned before it publishes a mode.
    #[must_use]
    pub fn fits(&self, bar_len: u64) -> bool {
        self.offset
            .checked_add(self.geometry.fb_size)
            .is_some_and(|end| end <= bar_len)
    }
}

/// A decoded (or to-be-encoded) `KFGP` descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Descriptor<'a> {
    /// Vendor id echo.
    pub vendor: u16,
    /// Device id echo.
    pub device: u16,
    /// The framebuffer.
    pub fb: BootFramebuffer,
    /// The monitor's EDID (0..=[`EDID_MAX`] bytes).
    pub edid: &'a [u8],
}

/// Why a descriptor is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescError {
    /// Fewer bytes than the header or the stated length.
    Short,
    /// No `KFGP` magic.
    BadMagic,
    /// A version this crate does not know.
    UnsupportedVersion(u16),
    /// The length field disagrees with the EDID length.
    BadLength,
    /// The bytes do not sum to 0.
    BadChecksum,
    /// A pixel format other than [`FORMAT_XRGB8888`].
    UnsupportedFormat(u8),
    /// A flag bit version 1 does not define.
    UnknownFlags(u16),
    /// The reserved field is not 0.
    NonZeroReserved,
    /// A BAR index above 5.
    BadBar(u8),
    /// The geometry is refused.
    Geometry(GeometryError),
    /// An EDID longer than [`EDID_MAX`].
    EdidTooLong(usize),
    /// `offset + G` overflows.
    FramebufferOverflow,
    /// The encode buffer is too small.
    BufferTooSmall,
    /// The ROM around the descriptor does not parse.
    Rom(ParseError),
    /// The ROM's first image is not an EFI image.
    NotEfiImage,
    /// The descriptor would run into the PE.
    OverlapsImage,
    /// The descriptor's ids are not PCIR's.
    IdMismatch,
}

impl From<GeometryError> for DescError {
    fn from(e: GeometryError) -> DescError {
        DescError::Geometry(e)
    }
}

impl fmt::Display for DescError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            DescError::Short => write!(f, "descriptor truncated"),
            DescError::BadMagic => write!(f, "no KFGP magic"),
            DescError::UnsupportedVersion(v) => {
                write!(f, "descriptor version {v} (this build knows {VERSION})")
            }
            DescError::BadLength => write!(f, "descriptor length disagrees with its EDID length"),
            DescError::BadChecksum => write!(f, "descriptor checksum"),
            DescError::UnsupportedFormat(x) => write!(f, "pixel format {x}"),
            DescError::UnknownFlags(x) => write!(f, "unknown flags {x:#x}"),
            DescError::NonZeroReserved => write!(f, "reserved field not zero"),
            DescError::BadBar(b) => write!(f, "BAR index {b}"),
            DescError::Geometry(g) => write!(f, "geometry: {g}"),
            DescError::EdidTooLong(n) => write!(f, "EDID of {n} bytes (at most {EDID_MAX})"),
            DescError::FramebufferOverflow => write!(f, "framebuffer offset + size overflows"),
            DescError::BufferTooSmall => write!(f, "encode buffer too small"),
            DescError::Rom(e) => write!(f, "ROM: {e}"),
            DescError::NotEfiImage => write!(f, "first image is not an EFI image"),
            DescError::OverlapsImage => write!(f, "descriptor runs into the PE"),
            DescError::IdMismatch => write!(f, "descriptor ids differ from PCIR's"),
        }
    }
}

/// The descriptor's offset in an image whose PCIR is at `pcir_offset` and `pcir_len` bytes long.
#[must_use]
pub fn descriptor_offset(pcir_offset: u16, pcir_len: u16) -> usize {
    (usize::from(pcir_offset) + usize::from(pcir_len) + 0xF) & !0xF
}

impl<'a> Descriptor<'a> {
    /// Bytes [`Descriptor::encode`] writes.
    #[must_use]
    pub fn encoded_len(&self) -> usize {
        HEADER_LEN + self.edid.len() + 1
    }

    fn validate(&self) -> Result<(), DescError> {
        if self.fb.bar > MAX_BAR {
            return Err(DescError::BadBar(self.fb.bar));
        }
        self.fb.geometry.check()?;
        if self.edid.len() > EDID_MAX {
            return Err(DescError::EdidTooLong(self.edid.len()));
        }
        if self
            .fb
            .offset
            .checked_add(self.fb.geometry.fb_size)
            .is_none()
        {
            return Err(DescError::FramebufferOverflow);
        }
        Ok(())
    }

    /// Encode into `out`; returns the bytes written. Refuses what [`Descriptor::decode`] would.
    pub fn encode(&self, out: &mut [u8]) -> Result<usize, DescError> {
        self.validate()?;
        let n = self.encoded_len();
        let out = out.get_mut(..n).ok_or(DescError::BufferTooSmall)?;
        out.fill(0);
        let g = self.fb.geometry;
        out[0..4].copy_from_slice(&MAGIC);
        out[0x04..0x06].copy_from_slice(&VERSION.to_le_bytes());
        // n ≤ MAX_LEN (validate bounded the EDID), so it fits u16.
        out[0x06..0x08].copy_from_slice(&(n as u16).to_le_bytes());
        out[0x08..0x0A].copy_from_slice(&self.vendor.to_le_bytes());
        out[0x0A..0x0C].copy_from_slice(&self.device.to_le_bytes());
        out[0x0C] = self.fb.bar;
        out[0x0D] = FORMAT_XRGB8888;
        out[0x10..0x18].copy_from_slice(&self.fb.offset.to_le_bytes());
        out[0x18..0x20].copy_from_slice(&g.fb_size.to_le_bytes());
        out[0x20..0x24].copy_from_slice(&g.width.to_le_bytes());
        out[0x24..0x28].copy_from_slice(&g.height.to_le_bytes());
        out[0x28..0x2C].copy_from_slice(&g.pitch.to_le_bytes());
        out[0x2C..0x2E].copy_from_slice(&(self.edid.len() as u16).to_le_bytes());
        out[HEADER_LEN..HEADER_LEN + self.edid.len()].copy_from_slice(self.edid);
        let sum = out[..n - 1].iter().fold(0u8, |a, b| a.wrapping_add(*b));
        out[n - 1] = sum.wrapping_neg();
        Ok(n)
    }

    /// Decode a descriptor starting at `bytes[0]`. Trailing bytes after its stated length are
    /// ignored.
    pub fn decode(bytes: &'a [u8]) -> Result<Descriptor<'a>, DescError> {
        if bytes.len() < HEADER_LEN + 1 {
            return Err(DescError::Short);
        }
        if bytes[0..4] != MAGIC {
            return Err(DescError::BadMagic);
        }
        // In range below: `bytes` holds at least HEADER_LEN + 1 bytes.
        let r16 = |o| le16(bytes, o).unwrap_or(0);
        let r32 = |o| le32(bytes, o).unwrap_or(0);
        let r64 = |o| le64(bytes, o).unwrap_or(0);
        let version = r16(0x04);
        if version != VERSION {
            return Err(DescError::UnsupportedVersion(version));
        }
        let len = usize::from(r16(0x06));
        let edid_len = usize::from(r16(0x2C));
        if edid_len > EDID_MAX {
            return Err(DescError::EdidTooLong(edid_len));
        }
        if len != HEADER_LEN + edid_len + 1 {
            return Err(DescError::BadLength);
        }
        let body = bytes.get(..len).ok_or(DescError::Short)?;
        if body.iter().fold(0u8, |a, b| a.wrapping_add(*b)) != 0 {
            return Err(DescError::BadChecksum);
        }
        if body[0x0D] != FORMAT_XRGB8888 {
            return Err(DescError::UnsupportedFormat(body[0x0D]));
        }
        let flags = r16(0x0E);
        if flags != 0 {
            return Err(DescError::UnknownFlags(flags));
        }
        if r16(0x2E) != 0 {
            return Err(DescError::NonZeroReserved);
        }
        let d = Descriptor {
            vendor: r16(0x08),
            device: r16(0x0A),
            fb: BootFramebuffer {
                bar: body[0x0C],
                offset: r64(0x10),
                geometry: Geometry {
                    width: r32(0x20),
                    height: r32(0x24),
                    pitch: r32(0x28),
                    fb_size: r64(0x18),
                },
            },
            edid: &body[HEADER_LEN..HEADER_LEN + edid_len],
        };
        d.validate()?;
        Ok(d)
    }

    /// Find and decode the descriptor of a packed ROM: in its first image, which must be an EFI
    /// image, after PCIR and before the PE, with ids equal to PCIR's.
    pub fn find(rom: &'a [u8]) -> Result<Descriptor<'a>, DescError> {
        let img = rom::image_at(rom, 0).map_err(DescError::Rom)?;
        if img.pcir.code_type != CODE_TYPE_EFI {
            return Err(DescError::NotEfiImage);
        }
        let efi = img.efi.ok_or(DescError::NotEfiImage)?;
        let at = descriptor_offset(img.pcir.offset, img.pcir.len);
        let end = usize::from(efi.image_header_offset);
        let slot = img.bytes.get(at..end).ok_or(DescError::OverlapsImage)?;
        let d = Descriptor::decode(slot).map_err(|e| {
            if e == DescError::Short {
                DescError::OverlapsImage
            } else {
                e
            }
        })?;
        if d.vendor != img.pcir.vendor || d.device != img.pcir.device {
            return Err(DescError::IdMismatch);
        }
        Ok(d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fb(width: u32, height: u32) -> BootFramebuffer {
        BootFramebuffer {
            bar: 1,
            offset: 0,
            geometry: Geometry::for_mode(width, height).unwrap(),
        }
    }

    #[test]
    fn the_1080p_geometry_is_the_designs_g() {
        let g = Geometry::for_mode(1920, 1080).unwrap();
        assert_eq!(g.pitch, 7680);
        assert_eq!(
            g.fb_size, 0x7F_0000,
            "V3_DISPLAY.md §4.11: G = 0x7F0000 at 1080p"
        );
    }

    #[test]
    fn the_stand_in_geometry_rounds_the_pitch() {
        // 1152·4 = 4608 = 18·256: already aligned. 1366·4 = 5464 → 5632.
        assert_eq!(Geometry::for_mode(1152, 648).unwrap().pitch, 4608);
        let g = Geometry::for_mode(1366, 768).unwrap();
        assert_eq!(g.pitch, 5632);
        assert_eq!(g.fb_size % FB_ALIGN, 0);
        assert!(g.fb_size >= g.min_fb_size());
    }

    #[test]
    fn bad_geometries_are_refused_by_name() {
        assert_eq!(Geometry::for_mode(0, 10), Err(GeometryError::ZeroSize));
        assert_eq!(
            Geometry::for_mode(20000, 10),
            Err(GeometryError::TooLarge {
                width: 20000,
                height: 10
            })
        );
        let g = Geometry {
            width: 100,
            height: 10,
            pitch: 300,
            fb_size: 1 << 20,
        };
        assert_eq!(g.check(), Err(GeometryError::PitchUnaligned { pitch: 300 }));
        let g = Geometry {
            width: 100,
            height: 10,
            pitch: 256,
            fb_size: 1 << 20,
        };
        assert_eq!(
            g.check(),
            Err(GeometryError::PitchTooSmall {
                pitch: 256,
                min: 400
            })
        );
        let g = Geometry {
            width: 100,
            height: 10,
            pitch: 512,
            fb_size: 5119,
        };
        assert_eq!(
            g.check(),
            Err(GeometryError::FramebufferTooSmall {
                fb_size: 5119,
                min: 5120
            })
        );
    }

    #[test]
    fn encode_decode_round_trip_with_an_edid() {
        let edid = [0xA5u8; 128];
        let d = Descriptor {
            vendor: 0x10de,
            device: 0x2504,
            fb: fb(1920, 1080),
            edid: &edid,
        };
        let mut buf = [0u8; MAX_LEN];
        let n = d.encode(&mut buf).unwrap();
        assert_eq!(n, HEADER_LEN + 128 + 1);
        assert_eq!(&buf[0..4], b"KFGP");
        assert_eq!(buf[..n].iter().fold(0u8, |a, b| a.wrapping_add(*b)), 0);
        assert_eq!(Descriptor::decode(&buf[..n]), Ok(d));
        // trailing bytes are ignored
        assert_eq!(Descriptor::decode(&buf), Ok(d));
    }

    #[test]
    fn every_corruption_is_refused_by_name() {
        let d = Descriptor {
            vendor: 0x10de,
            device: 0x2504,
            fb: fb(1920, 1080),
            edid: &[],
        };
        let mut good = [0u8; MAX_LEN];
        let n = d.encode(&mut good).unwrap();
        let fixsum = |b: &mut [u8]| {
            b[n - 1] = 0;
            let s = b[..n].iter().fold(0u8, |a, x| a.wrapping_add(*x));
            b[n - 1] = s.wrapping_neg();
        };
        let case = |f: &dyn Fn(&mut [u8]), want: DescError| {
            let mut b = good;
            f(&mut b);
            assert_eq!(Descriptor::decode(&b[..n]), Err(want));
        };
        case(&|b| b[0] = b'X', DescError::BadMagic);
        case(&|b| b[4] = 2, DescError::UnsupportedVersion(2));
        case(&|b| b[6] += 1, DescError::BadLength);
        case(&|b| b[0x20] ^= 1, DescError::BadChecksum);
        case(
            &|b| {
                b[0x0D] = 2;
                fixsum(b)
            },
            DescError::UnsupportedFormat(2),
        );
        case(
            &|b| {
                b[0x0E] = 1;
                fixsum(b)
            },
            DescError::UnknownFlags(1),
        );
        case(
            &|b| {
                b[0x2E] = 1;
                fixsum(b)
            },
            DescError::NonZeroReserved,
        );
        case(
            &|b| {
                b[0x0C] = 6;
                fixsum(b)
            },
            DescError::BadBar(6),
        );
        case(
            &|b| {
                b[0x18..0x20].copy_from_slice(&0x1000u64.to_le_bytes());
                fixsum(b)
            },
            DescError::Geometry(GeometryError::FramebufferTooSmall {
                fb_size: 0x1000,
                min: 7680 * 1080,
            }),
        );
        case(
            &|b| {
                b[0x10..0x18].copy_from_slice(&u64::MAX.to_le_bytes());
                fixsum(b)
            },
            DescError::FramebufferOverflow,
        );
        assert_eq!(
            Descriptor::decode(&good[..HEADER_LEN]),
            Err(DescError::Short)
        );
    }

    #[test]
    fn an_oversized_edid_is_refused_on_both_sides() {
        let edid = [0u8; EDID_MAX + 1];
        let d = Descriptor {
            vendor: 1,
            device: 2,
            fb: fb(640, 480),
            edid: &edid,
        };
        let mut buf = [0u8; MAX_LEN + 1];
        assert_eq!(
            d.encode(&mut buf),
            Err(DescError::EdidTooLong(EDID_MAX + 1))
        );
        let ok = Descriptor {
            edid: &edid[..EDID_MAX],
            ..d
        };
        let n = ok.encode(&mut buf).unwrap();
        buf[0x2C..0x2E].copy_from_slice(&((EDID_MAX + 1) as u16).to_le_bytes());
        assert_eq!(
            Descriptor::decode(&buf[..n]),
            Err(DescError::EdidTooLong(EDID_MAX + 1))
        );
        let mut small = [0u8; HEADER_LEN];
        assert_eq!(ok.encode(&mut small), Err(DescError::BufferTooSmall));
    }

    #[test]
    fn fits_checks_the_end_without_overflow() {
        let f = fb(1920, 1080);
        assert!(f.fits(0x7F_0000));
        assert!(!f.fits(0x7E_FFFF));
        let far = BootFramebuffer {
            offset: u64::MAX - 10,
            ..f
        };
        assert!(!far.fits(u64::MAX));
    }

    #[test]
    fn the_descriptor_slot_follows_pcir_on_16_bytes() {
        assert_eq!(descriptor_offset(0x1C, 0x1C), 0x40);
        assert_eq!(descriptor_offset(0x1C, 0x18), 0x40);
        assert_eq!(descriptor_offset(0x20, 0x1C), 0x40);
        assert_eq!(descriptor_offset(0x24, 0x1C), 0x40);
        assert_eq!(descriptor_offset(0x28, 0x1C), 0x50);
    }
}
