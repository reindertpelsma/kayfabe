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
    map: Vec<(u32, PixelFormat)>,
}

impl ScanFormats {
    /// Resolve for window class `win`; names the class lacks are simply not showable.
    #[must_use]
    pub fn resolve(t: &ClassTable, win: u32) -> ScanFormats {
        let names = [
            ("SET_PARAMS_FORMAT_A8R8G8B8", PixelFormat::Xrgb8888),
            ("SET_PARAMS_FORMAT_X8R8G8B8", PixelFormat::Xrgb8888),
            ("SET_PARAMS_FORMAT_A8B8G8R8", PixelFormat::Xbgr8888),
            ("SET_PARAMS_FORMAT_X8B8G8R8", PixelFormat::Xbgr8888),
            ("SET_PARAMS_FORMAT_R5G6B5", PixelFormat::Rgb565),
            ("SET_PARAMS_FORMAT_A2R10G10B10", PixelFormat::Xrgb2101010),
            ("SET_PARAMS_FORMAT_A2B10G10R10", PixelFormat::Xbgr2101010),
        ];
        ScanFormats {
            map: names
                .iter()
                .filter_map(|(n, f)| Some((t.v(win, n)?, *f)))
                .collect(),
        }
    }

    /// The console format of `SET_PARAMS.FORMAT` value `v`.
    #[must_use]
    pub fn of(&self, v: u32) -> Option<PixelFormat> {
        self.map.iter().find(|(x, _)| *x == v).map(|(_, f)| *f)
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
}
