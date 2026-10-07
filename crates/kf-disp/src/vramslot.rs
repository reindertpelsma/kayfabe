// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★ **The GPU-copy broker rung's frame object, as arithmetic** (`docs/design/V3_DISPLAY.md`
//! §8.11, `OWNER_RULINGS.md` §L) — pure, no I/O, the reference the pack kernel
//! (`cuda/display/kf_bl_pack.cu`) is held to.
//!
//! The finished frame — composed into the device staging frame, pitch-linear XRGB8888 at
//! `4·w` bytes a row, cursor included in grab mode (§O) — is packed by the GPU into a VRAM object
//! kayfabe allocated itself (a **slot**), in NVIDIA block-linear layout, and that object is
//! handed to the compositor as a dma-buf with `DRM_FORMAT_MOD_NVIDIA_BLOCK_LINEAR_2D`. Nothing of
//! the guest's — no surface, no slice of the store — is ever exported.
//!
//! - [`slot_geom`]: a `w`×`h` frame's GOBs per row, stride and block-linear extent at the slot's
//!   block height ([`SLOT_H_LOG2`], kayfabe's choice, independent of any guest surface);
//! - [`GobLayout`]: the byte order inside a 512-byte GOB as SETUP DATA (`THE_CONSTRAINTS.md` §21:
//!   a per-family difference found on a box is then a data change, not a kernel change);
//! - [`bl_chunk_origin`]: which `(x_bytes, y)` the 16 bytes at chunk `c` of GOB `g` hold — the
//!   exact inverse of [`crate::scanout::bl_offset`], which is what the kernel's one-warp-per-GOB,
//!   one-thread-per-16-bytes split computes;
//! - [`pack_reference`]: the whole slot as the CPU computes it, byte for byte, for the tests and
//!   for the box self-test that compares the GPU's pack with it before any slot is exported.

use crate::scanout::{GOB_BYTES, MAX_PIXELS};

/// The slot's block height: `2^4` = 16 GOBs (128 rows) — the modifier's `h`.
pub const SLOT_H_LOG2: u32 = 4;
/// Slot sizes are whole multiples of this: CUDA maps an imported object in units of its
/// allocation granularity (2 MiB on every GPU kayfabe drives), and RM rounds a large
/// allocation to its own large page anyway.
pub const SLOT_GRANULE: u64 = 2 << 20;
/// The largest edge a slot holds — the broker's `NVKVM_BROKER_MAX_DIM` (a larger frame would be
/// refused by every rung).
pub const SLOT_MAX_DIM: u32 = 8192;

/// ★ A `w`×`h` frame's block-linear shape at block height `2^h_log2` GOBs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotGeom {
    /// Pixels per row.
    pub width: u32,
    /// Rows.
    pub height: u32,
    /// GOBs per row (`ceil(4·w / 64)`).
    pub gobs_per_row: u32,
    /// The ATTACH's stride: `64 · gobs_per_row` bytes.
    pub stride: u32,
    /// log2 GOBs per block.
    pub h_log2: u32,
    /// Block rows (`ceil(h / (8 · 2^h_log2))`).
    pub block_rows: u32,
    /// Bytes the pack writes — every GOB of every block row.
    pub extent: u64,
}

impl SlotGeom {
    /// GOBs the pack writes (one warp each).
    #[must_use]
    pub fn gobs(&self) -> u64 {
        self.extent / GOB_BYTES
    }

    /// The slot size that holds it: [`Self::extent`] rounded up to [`SLOT_GRANULE`].
    #[must_use]
    pub fn slot_bytes(&self) -> u64 {
        self.extent.next_multiple_of(SLOT_GRANULE)
    }
}

/// ★ The shape of a `w`×`h` XR24 frame — `None` for an empty frame, an edge past
/// [`SLOT_MAX_DIM`], more than [`MAX_PIXELS`], or a block height past 5 (32 GOBs).
#[must_use]
pub fn slot_geom(width: u32, height: u32, h_log2: u32) -> Option<SlotGeom> {
    if width == 0
        || height == 0
        || width > SLOT_MAX_DIM
        || height > SLOT_MAX_DIM
        || u64::from(width) * u64::from(height) > MAX_PIXELS
        || h_log2 > 5
    {
        return None;
    }
    let gobs_per_row = (width * 4).div_ceil(64);
    let block_rows = height.div_ceil(8 << h_log2);
    Some(SlotGeom {
        width,
        height,
        gobs_per_row,
        stride: gobs_per_row * 64,
        h_log2,
        block_rows,
        extent: u64::from(block_rows) * u64::from(gobs_per_row) * (GOB_BYTES << h_log2),
    })
}

/// ★ The slot every frame up to 1920×1080 fits — what the five slots are provisioned at first
/// (`slot_geom(1920, 1080, 4)` = 8 847 360 B, rounded to 10 MiB).
pub const SLOT_CLASS0: u64 = 10 << 20;
/// ★ The slot every frame fits: the largest [`SlotGeom::slot_bytes`] over every `w`, `h` that
/// [`slot_geom`] accepts at [`SLOT_H_LOG2`] — derived by exhaustive search in this module's tests
/// (8081×1026: 37 306 368 B). A slot grows to this at most once.
pub const SLOT_MAX: u64 = 36 << 20;

/// ★ The byte order inside a 512-byte GOB, as setup data: the bit of the in-GOB offset that
/// carries each of `y[0]`, `y[1]`, `x[4]`, `y[2]`, `x[5]`. `x[3:0]` is always bits 3:0 (16
/// contiguous bytes), so the five name a permutation of bits 4..=8.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GobLayout {
    /// Bit of `y[0]`.
    pub y0: u32,
    /// Bit of `y[1]`.
    pub y1: u32,
    /// Bit of `x[4]`.
    pub x4: u32,
    /// Bit of `y[2]`.
    pub y2: u32,
    /// Bit of `x[5]`.
    pub x5: u32,
}

/// ★ `[measured m3b, 2026-09-30, GA106 / 580.159.04]` the order [`crate::scanout::bl_offset`]
/// encodes (it matched the X server's own screenshot in all 2 073 600 pixels). The only instance
/// until a box run measures another family (E6).
pub const GOB_GA106: GobLayout = GobLayout {
    y0: 4,
    y1: 5,
    x4: 6,
    y2: 7,
    x5: 8,
};

impl GobLayout {
    /// The five bits in kernel-parameter order (`y0, y1, x4, y2, x5`).
    #[must_use]
    pub fn bits(&self) -> [u32; 5] {
        [self.y0, self.y1, self.x4, self.y2, self.x5]
    }

    /// ★ A layout the kernel may be given: the five bits are a permutation of 4..=8 (any other
    /// value would place a chunk outside its GOB or two chunks on one place).
    ///
    /// # Errors
    /// The layout, by name.
    pub fn check(&self) -> Result<(), String> {
        let mut seen = 0u32;
        for b in self.bits() {
            if !(4..=8).contains(&b) || seen & (1 << b) != 0 {
                return Err(format!(
                    "GOB layout {self:?} is not a permutation of bits 4..=8"
                ));
            }
            seen |= 1 << b;
        }
        Ok(())
    }

    /// The in-GOB offset of byte `(x_bytes, y)` (`x_bytes < 64`, `y < 8` used).
    #[must_use]
    pub fn in_gob(&self, x_bytes: u64, y: u64) -> u64 {
        (x_bytes & 15)
            | ((y & 1) << self.y0)
            | (((y >> 1) & 1) << self.y1)
            | (((x_bytes >> 4) & 1) << self.x4)
            | (((y >> 2) & 1) << self.y2)
            | (((x_bytes >> 5) & 1) << self.x5)
    }
}

/// ★ [`crate::scanout::bl_offset`] with the GOB's byte order as a parameter (equal to it for
/// [`GOB_GA106`], pinned by a test).
#[must_use]
pub fn bl_offset_in(layout: &GobLayout, x_bytes: u64, y: u64, gobs_per_row: u64, bh: u32) -> u64 {
    let (gob_x, gob_y) = (x_bytes >> 6, y >> 3);
    let (block_y, in_block) = (gob_y >> bh, gob_y & ((1 << bh) - 1));
    let gob = ((block_y * gobs_per_row + gob_x) << bh) + in_block;
    gob * GOB_BYTES + layout.in_gob(x_bytes & 63, y & 7)
}

/// ★ The `(x_bytes, y)` of the first byte of chunk `c` (16 bytes at `16·c` of the GOB) of GOB
/// `g` (in memory order) — the inverse of [`bl_offset_in`] that the pack kernel computes per
/// thread: `x_bytes = 64·gob_x + 32·x5 + 16·x4`, `y = 8·gob_y + 4·y2 + 2·y1 + y0`, each bit read
/// from `16·c` at the layout's position.
#[must_use]
pub fn bl_chunk_origin(
    layout: &GobLayout,
    gobs_per_row: u64,
    bh: u32,
    g: u64,
    c: u64,
) -> (u64, u64) {
    let in_block = g & ((1 << bh) - 1);
    let t = g >> bh;
    let (gob_x, block_y) = (t % gobs_per_row, t / gobs_per_row);
    let gob_y = (block_y << bh) | in_block;
    let o = c << 4;
    let bit = |b: u32| (o >> b) & 1;
    let x_bytes = (gob_x << 6) | (bit(layout.x5) << 5) | (bit(layout.x4) << 4);
    let y = (gob_y << 3) | (bit(layout.y2) << 2) | (bit(layout.y1) << 1) | bit(layout.y0);
    (x_bytes, y)
}

/// ★ The slot's bytes as the CPU computes them from a pitch-linear staging frame (`4·w` bytes a
/// row, at least `4·w·h` long): every byte of [`SlotGeom::extent`] — a pixel inside the frame is
/// copied, every byte outside (the right and bottom padding of the last GOB column and block row)
/// is zero.
///
/// # Errors
/// A staging frame shorter than `4·w·h`, or a layout [`GobLayout::check`] refuses.
pub fn pack_reference(layout: &GobLayout, staging: &[u8], g: &SlotGeom) -> Result<Vec<u8>, String> {
    layout.check()?;
    let pitch = u64::from(g.width) * 4;
    let need = pitch * u64::from(g.height);
    if (staging.len() as u64) < need {
        return Err(format!(
            "a staging frame of {} bytes holds no {}x{} frame",
            staging.len(),
            g.width,
            g.height
        ));
    }
    let mut out = vec![0u8; usize::try_from(g.extent).map_err(|_| "extent".to_string())?];
    for y in 0..u64::from(g.height) {
        for xb in 0..pitch {
            let at = bl_offset_in(layout, xb, y, u64::from(g.gobs_per_row), g.h_log2);
            out[at as usize] = staging[(y * pitch + xb) as usize];
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanout::bl_offset;

    #[test]
    fn the_ga106_layout_is_bl_offset() {
        assert_eq!(GOB_GA106.check(), Ok(()));
        for bh in 0..=5 {
            for gpr in [1u64, 3, 30, 120] {
                for y in 0..(8u64 << bh) * 2 + 3 {
                    for x in 0..gpr * 64 {
                        assert_eq!(
                            bl_offset_in(&GOB_GA106, x, y, gpr, bh),
                            bl_offset(x, y, gpr, bh),
                            "({x}, {y}) gpr {gpr} bh {bh}"
                        );
                    }
                }
            }
        }
    }

    /// ★ The design's proof, exhaustive: for every block height 0..=5, odd and even widths, every
    /// GOB of the extent and all 32 chunks, `bl_offset(bl_chunk_origin(g, c)) = 512·g + 16·c` —
    /// the per-thread inverse IS the address function's inverse, so the pack writes each slot byte
    /// exactly once.
    #[test]
    fn the_chunk_origin_inverts_bl_offset_exactly() {
        for bh in 0..=5u32 {
            for w in [1u32, 15, 16, 17, 70, 129, 1366] {
                let h = 40 + bh * 17;
                let g = slot_geom(w, h, bh).unwrap();
                let gpr = u64::from(g.gobs_per_row);
                let mut hit = vec![false; g.extent as usize / 16];
                for gob in 0..g.gobs() {
                    for c in 0..32 {
                        let (x, y) = bl_chunk_origin(&GOB_GA106, gpr, bh, gob, c);
                        assert_eq!(x % 16, 0);
                        let at = bl_offset(x, y, gpr, bh);
                        assert_eq!(at, gob * 512 + c * 16, "w {w} bh {bh} gob {gob} c {c}");
                        hit[(at / 16) as usize] = true;
                    }
                }
                assert!(hit.iter().all(|h| *h), "every chunk of the extent written");
            }
        }
    }

    /// A permuted layout inverts too (the kernel takes it as data), and a bad one is refused.
    #[test]
    fn any_permutation_inverts_and_a_bad_layout_is_refused() {
        let other = GobLayout {
            y0: 4,
            y1: 6,
            x4: 5,
            y2: 7,
            x5: 8,
        };
        assert_eq!(other.check(), Ok(()));
        for gob in 0..40u64 {
            for c in 0..32 {
                let (x, y) = bl_chunk_origin(&other, 3, 2, gob, c);
                assert_eq!(bl_offset_in(&other, x, y, 3, 2), gob * 512 + c * 16);
            }
        }
        for bad in [
            GobLayout {
                y0: 4,
                y1: 4,
                ..GOB_GA106
            },
            GobLayout { x5: 9, ..GOB_GA106 },
            GobLayout { y0: 3, ..GOB_GA106 },
        ] {
            assert!(bad.check().is_err(), "{bad:?}");
        }
    }

    /// ★ `SLOT_MAX` derived, not assumed: every `w` 1..=8192 at its tallest `h` (the extent grows
    /// with `h`), and `SLOT_CLASS0` is 1080p's.
    #[test]
    fn slot_max_is_the_largest_slot_any_accepted_frame_needs() {
        let mut worst = (0u64, 0u32, 0u32);
        for w in 1..=SLOT_MAX_DIM {
            let h = SLOT_MAX_DIM.min(u32::try_from(MAX_PIXELS / u64::from(w)).unwrap());
            let g = slot_geom(w, h, SLOT_H_LOG2).unwrap();
            assert!(slot_geom(w, h + 1, SLOT_H_LOG2).is_none() || h == SLOT_MAX_DIM);
            worst = worst.max((g.slot_bytes(), w, h));
        }
        assert_eq!(worst.0, SLOT_MAX, "worst case {worst:?}");
        let big = slot_geom(8081, 1026, SLOT_H_LOG2).unwrap();
        assert_eq!(big.extent, 37_306_368);
        let hd = slot_geom(1920, 1080, SLOT_H_LOG2).unwrap();
        assert_eq!(
            (hd.gobs_per_row, hd.stride, hd.extent),
            (120, 7680, 8_847_360)
        );
        assert_eq!(hd.slot_bytes(), SLOT_CLASS0);
        assert_eq!(slot_geom(1366, 768, 4).unwrap().extent, 4_227_072);
        assert_eq!(slot_geom(3840, 2160, 4).unwrap().extent, 33_423_360);
        assert_eq!(slot_geom(3840, 2160, 4).unwrap().slot_bytes(), 32 << 20);
        for (w, h) in [(0, 1), (1, 0), (8193, 1), (1, 8193), (4000, 2160)] {
            assert_eq!(slot_geom(w, h, 4), None, "{w}x{h}");
        }
        assert_eq!(slot_geom(64, 64, 6), None);
    }

    /// The broker's own bounds pass for every slot shape: `4w <= stride <= 8w + 4096` and
    /// `stride * h <= extent <= slot bytes` (`nvkvm-pv nb_session_*.c` validators).
    #[test]
    fn every_slot_shape_passes_the_brokers_bounds() {
        for w in (1..=SLOT_MAX_DIM).step_by(7) {
            let h = SLOT_MAX_DIM.min(u32::try_from(MAX_PIXELS / u64::from(w)).unwrap());
            for h in [1, h / 2 + 1, h] {
                let g = slot_geom(w, h, SLOT_H_LOG2).unwrap();
                assert!(g.stride >= 4 * w && g.stride <= 8 * w + 4096, "{w}x{h}");
                assert!(u64::from(g.stride) * u64::from(h) <= g.extent);
                assert!(g.extent <= g.slot_bytes() && g.slot_bytes() <= SLOT_MAX);
            }
        }
    }

    /// The CPU reference: every pixel lands where `bl_offset` says, the padding is zero, and a
    /// short staging frame is refused.
    #[test]
    fn the_pack_reference_places_pixels_and_zeroes_padding() {
        let g = slot_geom(70, 40, 1).unwrap();
        let staging: Vec<u8> = (0..70 * 40 * 4).map(|i| (i % 251) as u8 | 1).collect();
        let out = pack_reference(&GOB_GA106, &staging, &g).unwrap();
        assert_eq!(out.len() as u64, g.extent);
        let mut covered = 0;
        for y in 0..40u64 {
            for x in 0..280u64 {
                let at = bl_offset(x, y, u64::from(g.gobs_per_row), 1) as usize;
                assert_eq!(out[at], staging[(y * 280 + x) as usize]);
                covered += 1;
            }
        }
        let nonzero = out.iter().filter(|b| **b != 0).count();
        assert_eq!(nonzero, covered, "every byte outside the frame is zero");
        assert!(pack_reference(&GOB_GA106, &staging[1..], &g).is_err());
    }
}
