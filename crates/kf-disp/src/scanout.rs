//! ★ M2 — turning what a head scans out ([`crate::engine::Scanout`], raw ARMED method values) into
//! ONE bounded copy of the guest's surface (`docs/design/V3_DISPLAY.md` §4.6).
//!
//! Pure arithmetic, no I/O: the display worker resolves the window's ISO context DMA, asks [`plan`]
//! what to copy, and hands the result to the GPU (`kf_cuda::display::DisplayGpu::scanout_pitch`) —
//! the CPU never reads the surface (`THE_CONSTRAINTS.md` §38).
//!
//! ⊘ Every value here is guest-chosen (hostile-guest bounds, `V3_DISPLAY.md` §6): the rectangle is
//! checked against the surface's own size, every byte of it against the context DMA's limit, the
//! output against [`MAX_PIXELS`], and each refusal says WHICH bound failed. A refusal never stops the
//! display — the flip still completes (the engine latched it); only the console keeps its last frame.

use crate::class::ClassTable;
use crate::engine::Scanout;
use crate::inst::{CtxDma, Target};

/// The largest frame the console takes (3840x2160): three page-locked frames of this size are the
/// plane's worst-case host memory (≈ 100 MB), and a guest cannot make it more.
pub const MAX_PIXELS: u64 = 3840 * 2160;

/// ★ A console pixel format — the FFI code the QEMU console maps to its own (`kf3.c`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum PixelFormat {
    /// 32 bpp, `0x00RRGGBB` in a little-endian word (NVDisplay `A8R8G8B8`, `X8R8G8B8`).
    Xrgb8888 = 1,
    /// 32 bpp, `0x00BBGGRR` (`A8B8G8R8`, `X8B8G8R8`).
    Xbgr8888 = 2,
    /// 16 bpp (`R5G6B5`).
    Rgb565 = 3,
    /// 32 bpp, 10 bits per channel (`A2R10G10B10`).
    Xrgb2101010 = 4,
    /// 32 bpp, 10 bits per channel (`A2B10G10R10`).
    Xbgr2101010 = 5,
}

impl PixelFormat {
    /// Bytes per pixel.
    #[must_use]
    pub fn bytes(self) -> u32 {
        match self {
            PixelFormat::Rgb565 => 2,
            _ => 4,
        }
    }
}

/// The window class's `SET_PARAMS_FORMAT` values the console can show, RESOLVED from the derived
/// class table (never hard-coded per family).
#[derive(Debug, Clone)]
pub struct ScanFormats {
    map: Vec<(u32, PixelFormat, bool)>,
}

impl ScanFormats {
    /// Resolve for window class `win`; names the class lacks are simply not showable.
    #[must_use]
    pub fn resolve(t: &ClassTable, win: u32) -> ScanFormats {
        let names = [
            ("SET_PARAMS_FORMAT_A8R8G8B8", PixelFormat::Xrgb8888, true),
            ("SET_PARAMS_FORMAT_X8R8G8B8", PixelFormat::Xrgb8888, false),
            ("SET_PARAMS_FORMAT_A8B8G8R8", PixelFormat::Xbgr8888, true),
            ("SET_PARAMS_FORMAT_X8B8G8R8", PixelFormat::Xbgr8888, false),
            ("SET_PARAMS_FORMAT_R5G6B5", PixelFormat::Rgb565, false),
            (
                "SET_PARAMS_FORMAT_A2R10G10B10",
                PixelFormat::Xrgb2101010,
                true,
            ),
            (
                "SET_PARAMS_FORMAT_A2B10G10R10",
                PixelFormat::Xbgr2101010,
                true,
            ),
        ];
        ScanFormats {
            map: names
                .iter()
                .filter_map(|(n, f, a)| Some((t.v(win, n)?, *f, *a)))
                .collect(),
        }
    }

    /// The console format of `SET_PARAMS.FORMAT` value `v`.
    #[must_use]
    pub fn of(&self, v: u32) -> Option<PixelFormat> {
        self.map
            .iter()
            .find(|(x, _, _)| *x == v)
            .map(|(_, f, _)| *f)
    }

    /// Does format `v` carry alpha (`A8…`, `A2…`)?
    #[must_use]
    pub fn has_alpha(&self, v: u32) -> bool {
        self.map.iter().any(|(x, _, a)| *x == v && *a)
    }
}

/// ★ One copy: `rows` rows of `row_bytes` from store offset `src` (surface pitch `src_pitch`) into a
/// tight `width` x `height` frame of `format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CopyPlan {
    /// Store offset of the rectangle's first pixel (pitch), or of the surface's first block
    /// (block-linear: the kernel walks the blocks from there).
    pub src: u64,
    /// The surface's pitch in bytes (pitch layout; the block-linear row of blocks is `layout`'s).
    pub src_pitch: u64,
    /// How the surface is laid out.
    pub layout: SurfaceLayout,
    /// Bytes per row (`width * bpp`) — also the frame's pitch.
    pub row_bytes: u64,
    /// Rows (= `height`).
    pub rows: u32,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Pixel format.
    pub format: PixelFormat,
}

impl CopyPlan {
    /// Bytes of the frame it fills.
    #[must_use]
    pub fn frame_bytes(&self) -> u64 {
        self.row_bytes * u64::from(self.rows)
    }
}

/// ★ The surface's memory layout, as the copy walks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceLayout {
    /// Rows `src_pitch` bytes apart — one 2D copy.
    Pitch,
    /// NVIDIA block-linear: 64-byte x 8-row GOBs, `2^block_height_log2` GOBs per block (one GOB
    /// wide), blocks row-major `gobs_per_row` to a row — a GPU kernel un-swizzles it
    /// ([`bl_offset`] is its reference).
    BlockLinear {
        /// The surface's width in GOBs (`SET_PLANAR_STORAGE.PITCH` in block units).
        gobs_per_row: u32,
        /// `SET_STORAGE.BLOCK_HEIGHT` (0..=5).
        block_height_log2: u32,
        /// The rectangle's first byte column (`x * bpp`, a multiple of 4).
        x0_bytes: u32,
        /// The rectangle's first row.
        y0: u32,
        /// Bytes from `src` the kernel may read: whole blocks down to the rectangle's last row.
        extent: u64,
    },
}

/// Bytes in a GOB (64 x 8).
pub const GOB_BYTES: u64 = 512;

/// ★ The byte offset of surface byte `(x_bytes, y)` in a block-linear surface `gobs_per_row` GOBs
/// wide with `2^bh` GOBs per block — the reference the scanout kernel (`cuda/display/kf_scanout.ptx`)
/// implements. Inside a 64-byte x 8-row GOB, from bit 0 up: `x[3:0]`, `y[1:0]`, `x[4]`, `y[2]`,
/// `x[5]` — 32-byte sectors of 16 bytes x 2 rows, stacked two high before the next 16 bytes across.
/// GOBs stack `2^bh` high into a block; blocks run row-major.
///
/// ⊘ `[measured m3b, 2026-09-30, GA106 / 580.159.04]` NOT the often-quoted Tegra X1 order
/// (`x[4]` at bit 5, `y[1]` at bit 6): with that order the console's copy of the NVIDIA X driver's
/// block-linear desktop differed from the X server's own root-window screenshot in 23 208 pixels,
/// every one a 16-byte chunk displaced by (±16 bytes, ∓2 rows); with bits 5 and 6 exchanged it
/// matched in all 2 073 600.
#[must_use]
pub fn bl_offset(x_bytes: u64, y: u64, gobs_per_row: u64, bh: u32) -> u64 {
    let (gob_x, gob_y) = (x_bytes >> 6, y >> 3);
    let (block_y, in_block) = (gob_y >> bh, gob_y & ((1 << bh) - 1));
    let gob = ((block_y * gobs_per_row + gob_x) << bh) + in_block;
    let in_gob = ((x_bytes & 32) << 3)
        | ((y & 4) << 5)
        | ((x_bytes & 16) << 2)
        | ((y & 3) << 4)
        | (x_bytes & 15);
    gob * GOB_BYTES + in_gob
}

/// Why a scanout cannot be copied (the bound that failed, by name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused(pub String);

/// ★ Plan the copy of scanout `s` whose ISO context DMA resolved to `dma`.
///
/// # Errors
/// [`Refused`], naming the bound: a system-memory surface, a format the console has no match for,
/// an empty or oversized rectangle, a rectangle outside the surface, a block-linear geometry the
/// kernel cannot walk, or any byte outside the context DMA.
pub fn plan(s: &Scanout, dma: &CtxDma, formats: &ScanFormats) -> Result<CopyPlan, Refused> {
    let no = |why: String| {
        Err(Refused(format!(
            "window {} head {}: {why}",
            s.window, s.head
        )))
    };
    if dma.target != Target::Vidmem {
        return no("a system-memory surface (the console shows video memory only)".into());
    }
    let Some(format) = formats.of(s.format) else {
        return no(format!(
            "SET_PARAMS.FORMAT {:#x} has no console format",
            s.format
        ));
    };
    if s.width == 0 || s.height == 0 {
        return no(format!(
            "an empty source rectangle {}x{}",
            s.width, s.height
        ));
    }
    if u64::from(s.width) * u64::from(s.height) > MAX_PIXELS {
        return no(format!(
            "{}x{} is larger than the console's {MAX_PIXELS} pixels",
            s.width, s.height
        ));
    }
    // the rectangle inside the surface's own size
    if u64::from(s.x) + u64::from(s.width) > u64::from(s.surface_width)
        || u64::from(s.y) + u64::from(s.height) > u64::from(s.surface_height)
    {
        return no(format!(
            "rectangle {}x{}+{}+{} leaves the {}x{} surface",
            s.width, s.height, s.x, s.y, s.surface_width, s.surface_height
        ));
    }
    let bpp = u64::from(format.bytes());
    let src_pitch = u64::from(s.pitch) * 64;
    let row_bytes = u64::from(s.width) * bpp;
    if dma.block_linear {
        return plan_block_linear(s, dma, format, row_bytes).or_else(no);
    }
    if row_bytes > src_pitch {
        return no(format!(
            "a {row_bytes}-byte row is wider than the {src_pitch}-byte pitch"
        ));
    }
    let first = s.offset + u64::from(s.y) * src_pitch + u64::from(s.x) * bpp;
    let span = u64::from(s.height - 1) * src_pitch + row_bytes;
    let Some(src) = dma.span(first, span) else {
        return no(format!(
            "[{first:#x}, +{span:#x}) leaves context DMA {:#x}..={:#x}",
            dma.base, dma.limit
        ));
    };
    Ok(CopyPlan {
        src,
        src_pitch,
        layout: SurfaceLayout::Pitch,
        row_bytes,
        rows: s.height,
        width: s.width,
        height: s.height,
        format,
    })
}

/// The block-linear half of [`plan`] (the rectangle is already inside the surface's own size).
fn plan_block_linear(
    s: &Scanout,
    dma: &CtxDma,
    format: PixelFormat,
    row_bytes: u64,
) -> Result<CopyPlan, String> {
    let bh = s.block_height_log2;
    if bh > 5 {
        return Err(format!("SET_STORAGE.BLOCK_HEIGHT {bh} is not 1..32 GOBs"));
    }
    let gobs_per_row = u64::from(s.pitch);
    let x0_bytes = u64::from(s.x) * u64::from(format.bytes());
    // the kernel moves 4-byte words
    if x0_bytes % 4 != 0 || row_bytes % 4 != 0 {
        return Err(format!(
            "a {row_bytes}-byte row from byte {x0_bytes} is not whole 4-byte words"
        ));
    }
    if x0_bytes + row_bytes > gobs_per_row * 64 {
        return Err(format!(
            "bytes {x0_bytes}..{} leave the {gobs_per_row}-GOB-wide surface",
            x0_bytes + row_bytes
        ));
    }
    let rows_per_block = 8u64 << bh;
    let block_rows = (u64::from(s.y) + u64::from(s.height)).div_ceil(rows_per_block);
    let extent = block_rows * gobs_per_row * (GOB_BYTES << bh);
    let Some(src) = dma.span(s.offset, extent) else {
        return Err(format!(
            "[{:#x}, +{extent:#x}) (block-linear) leaves context DMA {:#x}..={:#x}",
            s.offset, dma.base, dma.limit
        ));
    };
    Ok(CopyPlan {
        src,
        src_pitch: 0,
        layout: SurfaceLayout::BlockLinear {
            gobs_per_row: s.pitch,
            block_height_log2: bh,
            x0_bytes: u32::try_from(x0_bytes).map_err(|_| "x0".to_string())?,
            y0: s.y,
            extent,
        },
        row_bytes,
        rows: s.height,
        width: s.width,
        height: s.height,
        format,
    })
}

/// ★ One window's program for the compose kernel (`cuda/display/kf_scanout.ptx`, `kf_compose`):
/// where to read (bounded), where it lands in the head's frame (clipped), and how it blends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerPlan {
    /// The window.
    pub window: u32,
    /// Store offset the kernel reads from: the rectangle's first pixel (pitch) or the surface's
    /// first block (block-linear).
    pub src: u64,
    /// Bytes from `src` the kernel may read.
    pub extent: u64,
    /// Block-linear (else pitch).
    pub block_linear: bool,
    /// Pitch in bytes (pitch), or GOBs per row (block-linear).
    pub pitch: u32,
    /// log2 GOBs per block (block-linear).
    pub block_height_log2: u32,
    /// The rectangle's first byte column and row (block-linear; 0 for pitch — `src` is there).
    pub x0_bytes: u32,
    /// Its first row.
    pub y0: u32,
    /// Pixels per row, after clipping to the output size and the frame.
    pub width: u32,
    /// Rows, likewise.
    pub rows: u32,
    /// Where the rectangle's first pixel lands in the frame.
    pub ox: u32,
    /// Its row in the frame.
    pub oy: u32,
    /// `bit0` the source has alpha, `bit1` swap red/blue, `bit2` opaque (store the source word).
    pub flags: u32,
    /// Source factor `as + bs * alpha / 255` and destination factor `ad + bd * alpha / 255`.
    pub a_s: i32,
    /// See `a_s`.
    pub b_s: i32,
    /// See `a_s`.
    pub a_d: i32,
    /// See `a_s`.
    pub b_d: i32,
}

/// A `SET_COMPOSITION_FACTOR_SELECT` selector as `(a, b)` of `a + b * source_alpha / 255` (both
/// selector tables share their encodings: `clc67e.h:215-264`); the frame below is opaque, so a
/// `…_TIMES_DST` factor is its constant.
fn factor(sel: u32, k1: i32, k2: i32) -> Option<(i32, i32)> {
    Some(match sel {
        0 => (0, 0),        // ZERO
        1 => (255, 0),      // ONE
        2 => (k1, 0),       // K1
        3 => (k2, 0),       // K2
        4 => (255 - k1, 0), // NEG_K1
        5 => (0, k1),       // K1_TIMES_SRC
        6 => (k1, 0),       // K1_TIMES_DST
        7 => (255, -k1),    // NEG_K1_TIMES_SRC
        8 => (255 - k1, 0), // NEG_K1_TIMES_DST
        _ => return None,
    })
}

/// ★ Plan window `s` of a `fw` x `fh` composition: [`plan`]'s bounds, then its position, clipping
/// and blend. `Ok(None)` for a window wholly outside the frame.
///
/// # Errors
/// [`plan`]'s refusals; a format the compose kernel cannot read (it moves 32-bit pixels); a
/// composition factor it does not know.
pub fn plan_layer(
    s: &Scanout,
    dma: &CtxDma,
    formats: &ScanFormats,
    fw: u32,
    fh: u32,
) -> Result<Option<LayerPlan>, Refused> {
    let c = plan(s, dma, formats)?;
    let no = |why: String| {
        Err(Refused(format!(
            "window {} head {}: {why}",
            s.window, s.head
        )))
    };
    let swap = match c.format {
        PixelFormat::Xrgb8888 => false,
        PixelFormat::Xbgr8888 => true,
        f => {
            return no(format!(
                "{f:?} is not composable (the kernel moves 32-bit 8888 pixels)"
            ));
        }
    };
    if s.out_x >= fw || s.out_y >= fh {
        return Ok(None);
    }
    let clip = |n: u32, out: u32, room: u32| n.min(if out == 0 { n } else { out }).min(room);
    let width = clip(c.width, s.out_width, fw - s.out_x);
    let rows = clip(c.rows, s.out_height, fh - s.out_y);
    if width == 0 || rows == 0 {
        return Ok(None);
    }
    let (k1, k2) = (
        i32::try_from(s.k1.min(255)).unwrap_or(255),
        i32::try_from(s.k2.min(255)).unwrap_or(255),
    );
    // ⊘ a window whose factor word was never programmed (both ZERO) would erase what lies below;
    // NVKMS programs it with every flip (`UpdateComposition`), so an all-zero word is read as opaque
    let (src_sel, dst_sel) = if s.src_factor == 0 && s.dst_factor == 0 {
        (1, 0)
    } else {
        (s.src_factor, s.dst_factor)
    };
    let (Some((a_s, b_s)), Some((a_d, b_d))) = (factor(src_sel, k1, k2), factor(dst_sel, k1, k2))
    else {
        return no(format!(
            "composition factors {:#x}/{:#x} are not known",
            s.src_factor, s.dst_factor
        ));
    };
    let alpha = formats.has_alpha(s.format);
    let opaque = (a_s, b_s, a_d, b_d) == (255, 0, 0, 0);
    let flags = u32::from(alpha) | (u32::from(swap) << 1) | (u32::from(opaque) << 2);
    let (block_linear, pitch, bh, x0_bytes, y0, extent) = match c.layout {
        SurfaceLayout::Pitch => (
            false,
            u32::try_from(c.src_pitch).map_err(|_| Refused("pitch".into()))?,
            0,
            0,
            0,
            u64::from(rows - 1) * c.src_pitch + u64::from(width) * 4,
        ),
        SurfaceLayout::BlockLinear {
            gobs_per_row,
            block_height_log2,
            x0_bytes,
            y0,
            extent,
        } => (true, gobs_per_row, block_height_log2, x0_bytes, y0, extent),
    };
    Ok(Some(LayerPlan {
        window: s.window,
        src: c.src,
        extent,
        block_linear,
        pitch,
        block_height_log2: bh,
        x0_bytes,
        y0,
        width,
        rows,
        ox: s.out_x,
        oy: s.out_y,
        flags,
        a_s,
        b_s,
        a_d,
        b_d,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn formats() -> ScanFormats {
        ScanFormats::resolve(crate::class::for_version("580.159.04").unwrap(), 0xC67E)
    }

    fn fb1080() -> Scanout {
        Scanout {
            window: 6,
            head: 3,
            chn: 7,
            client: 0xc1d0_0015,
            handle: 0x1008c,
            offset: 0x1_0000,
            width: 1920,
            height: 1080,
            x: 0,
            y: 0,
            surface_width: 1920,
            surface_height: 1080,
            pitch: 7680 / 64,
            block_height_log2: 0,
            format: 0xE6,
            out_x: 0,
            out_y: 0,
            out_width: 1920,
            out_height: 1080,
            depth: 255,
            k1: 255,
            k2: 0,
            src_factor: 1,
            dst_factor: 0,
        }
    }

    fn vid(base: u64, bytes: u64) -> CtxDma {
        CtxDma {
            target: Target::Vidmem,
            base,
            limit: base + bytes - 1,
            block_linear: false,
            writable: true,
        }
    }

    /// ★ The probe's 1080p dumb buffer: one tight 7680-byte-pitch copy of 1080 rows, from the
    /// context DMA's base + `SET_OFFSET`, as `X8R8G8B8`.
    #[test]
    fn a_1080p_pitch_surface_is_one_tight_copy() {
        let p = plan(&fb1080(), &vid(0x4000_0000, 64 << 20), &formats()).unwrap();
        assert_eq!(p.src, 0x4001_0000);
        assert_eq!((p.src_pitch, p.row_bytes, p.rows), (7680, 7680, 1080));
        assert_eq!(
            (p.width, p.height, p.format),
            (1920, 1080, PixelFormat::Xrgb8888)
        );
        assert_eq!(p.frame_bytes(), 7680 * 1080);
    }

    /// A panned viewport inside a larger surface starts at its point and keeps the surface's pitch.
    #[test]
    fn a_viewport_inside_a_larger_surface_keeps_the_surface_pitch() {
        let mut s = fb1080();
        s.surface_width = 2560;
        s.surface_height = 1440;
        s.pitch = 10240 / 64;
        s.x = 16;
        s.y = 2;
        let p = plan(&s, &vid(0, 64 << 20), &formats()).unwrap();
        assert_eq!(p.src, 0x1_0000 + 2 * 10240 + 16 * 4);
        assert_eq!((p.src_pitch, p.row_bytes), (10240, 7680));
    }

    /// ⊘ Hostile values, each refused by the bound it breaks — never clamped into something copyable.
    #[test]
    fn every_hostile_rectangle_is_refused_by_name() {
        let f = formats();
        let dma = vid(0x4000_0000, 8 << 20);
        let why = |s: Scanout, d: CtxDma| plan(&s, &d, &f).unwrap_err().0;
        // one byte past the context DMA's limit
        let mut s = fb1080();
        s.offset = (8 << 20) - 7680 * 1080 + 1;
        assert!(
            why(s, dma).contains("leaves context DMA"),
            "{}",
            why(s, dma)
        );
        // a rectangle past the surface
        let mut s = fb1080();
        s.y = 1;
        assert!(why(s, dma).contains("leaves the 1920x1080 surface"));
        // a pitch narrower than a row
        let mut s = fb1080();
        s.pitch = 1;
        assert!(why(s, dma).contains("wider than"));
        // zero and enormous
        let mut s = fb1080();
        s.height = 0;
        assert!(why(s, dma).contains("empty"));
        let mut s = fb1080();
        (s.width, s.height, s.surface_width, s.surface_height) = (8192, 8192, 8192, 8192);
        s.pitch = 8192 * 4 / 64;
        assert!(why(s, dma).contains("larger than the console"));
        // an unknown format, a block-linear surface, a system-memory surface
        let mut s = fb1080();
        s.format = 0x1E;
        assert!(why(s, dma).contains("no console format"));
        let mut d = dma;
        d.target = Target::Sysmem;
        assert!(why(fb1080(), d).contains("system-memory"));
    }

    /// The formats come from the derived table: both 8888 orders, 565 and the 10-bit pair map; an
    /// indexed format does not.
    #[test]
    fn console_formats_are_the_derived_values() {
        let f = formats();
        assert_eq!(f.of(0xCF), Some(PixelFormat::Xrgb8888), "A8R8G8B8");
        assert_eq!(f.of(0xE6), Some(PixelFormat::Xrgb8888), "X8R8G8B8");
        assert_eq!(f.of(0xD5), Some(PixelFormat::Xbgr8888), "A8B8G8R8");
        assert_eq!(f.of(0xE8), Some(PixelFormat::Rgb565), "R5G6B5");
        assert_eq!(f.of(0xDF), Some(PixelFormat::Xrgb2101010));
        assert_eq!(f.of(0x1E), None, "I8");
    }

    /// ★ The GOB swizzle, bit by bit (`x[3:0] y[1:0] x[4] y[2] x[5]`, measured in m3b), and GOBs
    /// stacked into blocks.
    #[test]
    fn the_block_linear_reference_is_the_nvidia_gob_swizzle() {
        assert_eq!(bl_offset(0, 0, 1, 0), 0);
        assert_eq!(bl_offset(15, 0, 1, 0), 15, "x[3:0] -> bits 3:0");
        assert_eq!(bl_offset(0, 1, 1, 0), 16, "y[0] -> bit 4");
        assert_eq!(bl_offset(0, 2, 1, 0), 32, "y[1] -> bit 5");
        assert_eq!(bl_offset(16, 0, 1, 0), 64, "x[4] -> bit 6");
        assert_eq!(bl_offset(0, 4, 1, 0), 128, "y[2] -> bit 7");
        assert_eq!(bl_offset(32, 0, 1, 0), 256, "x[5] -> bit 8");
        assert_eq!(bl_offset(63, 7, 1, 0), 511, "the GOB's last byte");
        // 16-GOB blocks: the next GOB DOWN is the next GOB in memory, the next one ACROSS is a block away
        assert_eq!(bl_offset(0, 8, 30, 4), 512);
        assert_eq!(bl_offset(64, 0, 30, 4), 16 * 512);
        assert_eq!(
            bl_offset(0, 128, 30, 4),
            30 * 16 * 512,
            "the next row of blocks"
        );
    }

    /// Every byte of a whole-block surface has exactly one address, and the addresses are the whole
    /// surface — for every block height.
    #[test]
    fn the_block_linear_reference_is_a_bijection() {
        for bh in 0..=5u32 {
            let (gpr, block_rows) = (3u64, 2u64);
            let rows = block_rows * (8 << bh);
            let total = gpr * 64 * rows;
            let mut seen = vec![false; total as usize];
            for y in 0..rows {
                for x in 0..gpr * 64 {
                    let o = bl_offset(x, y, gpr, bh) as usize;
                    assert!(o < seen.len() && !seen[o], "bh {bh}: ({x},{y}) -> {o}");
                    seen[o] = true;
                }
            }
            assert!(seen.iter().all(|b| *b), "bh {bh}: a hole");
        }
    }

    /// ★ A 1080p block-linear surface (NVKMS's 16-GOB blocks): the kernel reads whole blocks from the
    /// surface's start, 9 block rows of 120 GOBs; every refusal names its bound.
    #[test]
    fn a_block_linear_surface_is_planned_as_whole_blocks() {
        let f = formats();
        let mut s = fb1080();
        s.pitch = 7680 / 64;
        s.block_height_log2 = 4;
        let mut dma = vid(0x4000_0000, 16 << 20);
        dma.block_linear = true;
        let p = plan(&s, &dma, &f).unwrap();
        assert_eq!(p.src, 0x4001_0000, "the surface's first block, not a pixel");
        assert_eq!(
            p.layout,
            SurfaceLayout::BlockLinear {
                gobs_per_row: 120,
                block_height_log2: 4,
                x0_bytes: 0,
                y0: 0,
                extent: 9 * 120 * 16 * 512,
            }
        );
        assert_eq!((p.row_bytes, p.rows), (7680, 1080));
        let why = |s: Scanout, d: CtxDma| plan(&s, &d, &f).unwrap_err().0;
        let mut t = s;
        t.block_height_log2 = 6;
        assert!(why(t, dma).contains("BLOCK_HEIGHT"));
        let mut t = s;
        t.pitch = 119;
        assert!(why(t, dma).contains("GOB-wide"), "{}", why(t, dma));
        let mut d = dma;
        d.limit = d.base + 0x1_0000 + 9 * 120 * 16 * 512 - 2;
        assert!(why(s, d).contains("leaves context DMA"));
        let mut t = s;
        t.format = 0xE8; // R5G6B5
        t.x = 1;
        t.width = 1919;
        assert!(why(t, dma).contains("4-byte words"));
    }

    /// ★ M3 composition: an opaque full-screen window is one opaque store per pixel; a premultiplied
    /// ARGB overlay blends (`src*K1 + dst*(1 - K1*a)`), lands at its `POINT_OUT` and is clipped to
    /// its output size and the frame; a window outside the frame is nothing.
    #[test]
    fn layers_are_positioned_clipped_and_blended() {
        let f = formats();
        let dma = vid(0x4000_0000, 64 << 20);
        let full = plan_layer(&fb1080(), &dma, &f, 1920, 1080)
            .unwrap()
            .unwrap();
        assert_eq!(
            (full.width, full.rows, full.ox, full.oy),
            (1920, 1080, 0, 0)
        );
        assert_eq!(full.flags, 4, "opaque, no alpha, no swap");
        assert_eq!((full.src, full.pitch), (0x4001_0000, 7680));
        assert_eq!(full.extent, 1079 * 7680 + 7680);
        let mut o = fb1080();
        (o.width, o.height, o.out_width, o.out_height) = (250, 250, 250, 250);
        (o.out_x, o.out_y, o.format) = (100, 50, 0xCF); // A8R8G8B8
        (o.src_factor, o.dst_factor, o.k1) = (2, 7, 255); // K1 / NEG_K1_TIMES_SRC: premultiplied
        let l = plan_layer(&o, &dma, &f, 1920, 1080).unwrap().unwrap();
        assert_eq!((l.ox, l.oy, l.width, l.rows), (100, 50, 250, 250));
        assert_eq!(l.flags, 1, "alpha, blended");
        assert_eq!((l.a_s, l.b_s, l.a_d, l.b_d), (255, 0, 255, -255));
        (o.out_x, o.out_y) = (1800, 1000);
        let c = plan_layer(&o, &dma, &f, 1920, 1080).unwrap().unwrap();
        assert_eq!((c.width, c.rows), (120, 80), "clipped to the frame");
        (o.out_x, o.out_y) = (1920, 0);
        assert_eq!(plan_layer(&o, &dma, &f, 1920, 1080), Ok(None));
        (o.out_x, o.src_factor) = (0, 9);
        assert!(
            plan_layer(&o, &dma, &f, 1920, 1080)
                .unwrap_err()
                .0
                .contains("not known")
        );
        let mut r = fb1080();
        r.format = 0xE8; // R5G6B5
        r.pitch = 3840 / 64;
        assert!(
            plan_layer(&r, &dma, &f, 1920, 1080)
                .unwrap_err()
                .0
                .contains("composable")
        );
    }
}
