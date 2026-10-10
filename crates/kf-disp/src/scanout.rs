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
use crate::engine::{CursorScan, Scanout};
use crate::inst::{CtxDma, Target};

/// The largest frame the console takes (3840x2160). Three page-locked frames of this size are the
/// plane's worst-case host memory with the display broker off (≈ 100 MB). ⊘ With `display-broker`
/// on there are five slots and a slot that grew keeps its 1080p backing retired (descriptors are never
/// closed): 5 × (7.9 + 31.6) MiB ≈ 198 MiB (`docs/design/V3_DISPLAY.md` §8.3). A guest cannot make it
/// more.
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
    /// ★ The semi-planar 8-bit YUV formats (a luma plane and one interleaved chroma plane), by the
    /// class's own format names.
    yuv: Vec<(u32, YuvFormat)>,
}

/// ★ A semi-planar 8-bit YUV window format the console composes: plane 0 is `Y8`, plane 1 holds
/// interleaved chroma byte pairs, one pair per `2^sub_x_log2` x `2^sub_y_log2` luma pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct YuvFormat {
    /// log2 of the horizontal chroma subsampling (4:2:0 and 4:2:2: 1; 4:4:4: 0).
    pub sub_x_log2: u32,
    /// log2 of the vertical chroma subsampling (4:2:0: 1).
    pub sub_y_log2: u32,
    /// The chroma pair is V then U (`Y8___V8U8`), else U then V.
    pub vu_first: bool,
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
        // ★ derived from the class's own names, like the RGB table: a family without one simply
        // has no console YUV format (and the window is left out of the console copy, by name)
        let yuv_names = [
            ("SET_PARAMS_FORMAT_Y8___U8V8_N444", 0, 0, false),
            ("SET_PARAMS_FORMAT_Y8___U8V8_N422", 1, 0, false),
            ("SET_PARAMS_FORMAT_Y8___V8U8_N420", 1, 1, true),
        ];
        ScanFormats {
            map: names
                .iter()
                .filter_map(|(n, f, a)| Some((t.v(win, n)?, *f, *a)))
                .collect(),
            yuv: yuv_names
                .iter()
                .filter_map(|&(n, sx, sy, vu)| {
                    Some((
                        t.v(win, n)?,
                        YuvFormat {
                            sub_x_log2: sx,
                            sub_y_log2: sy,
                            vu_first: vu,
                        },
                    ))
                })
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

    /// The console YUV format of `SET_PARAMS.FORMAT` value `v`, if it is one of the semi-planar
    /// 8-bit formats [`plan_yuv`] composes.
    #[must_use]
    pub fn yuv_of(&self, v: u32) -> Option<YuvFormat> {
        self.yuv.iter().find(|(x, _)| *x == v).map(|(_, f)| *f)
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
    if !x0_bytes.is_multiple_of(4) || !row_bytes.is_multiple_of(4) {
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

/// [`LayerPlan::flags`] — the compose kernel's (`cuda/display/kf_scanout.ptx`, `kf_compose`): the
/// source has alpha (else it reads as 255) …
pub const COMPOSE_ALPHA: u32 = 1;
/// … swap red and blue (an `A8B8G8R8`/`X8B8G8R8` source) …
pub const COMPOSE_SWAP_RB: u32 = 2;
/// … opaque: store the source word …
pub const COMPOSE_OPAQUE: u32 = 4;
/// … ★ XOR (`OWNER_RULINGS.md` §O, 2026-10-04): `out = below XOR (source & 0x00ffffff)` — the
/// cursor composition `MODE_XOR` (`ogkm-580: clc37d.h:859-861`); alpha and the factors play no
/// part (see [`plan_cursor`] for why this is a definition, not a measurement).
pub const COMPOSE_XOR: u32 = 8;
/// Every flag the kernel knows; a layer carrying another bit is refused before a launch.
pub const COMPOSE_FLAGS: u32 = COMPOSE_ALPHA | COMPOSE_SWAP_RB | COMPOSE_OPAQUE | COMPOSE_XOR;

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
    /// [`COMPOSE_ALPHA`], [`COMPOSE_SWAP_RB`], [`COMPOSE_OPAQUE`], [`COMPOSE_XOR`].
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
    let flags = if alpha { COMPOSE_ALPHA } else { 0 }
        | if swap { COMPOSE_SWAP_RB } else { 0 }
        | if opaque { COMPOSE_OPAQUE } else { 0 };
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

/// ★ One semi-planar YUV window's program for the YUV compose kernel (`cuda/display/kf_yuv.cu`):
/// both planes' bounded reads, the source rectangle, and the (scaled) destination rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct YuvPlan {
    /// The window.
    pub window: u32,
    /// Store offset of the luma plane's first byte, and the bytes the kernel may read from it.
    pub y_src: u64,
    /// See `y_src`.
    pub y_extent: u64,
    /// The same for the chroma plane.
    pub c_src: u64,
    /// See `c_src`.
    pub c_extent: u64,
    /// Block-linear (else pitch) — both planes.
    pub block_linear: bool,
    /// Luma pitch in bytes, or GOBs per row.
    pub y_pitch: u32,
    /// Chroma pitch in bytes, or GOBs per row.
    pub c_pitch: u32,
    /// log2 GOBs per block (block-linear).
    pub block_height_log2: u32,
    /// The source rectangle's origin in the luma plane, and its size.
    pub sx0: u32,
    /// See `sx0`.
    pub sy0: u32,
    /// See `sx0`.
    pub sw: u32,
    /// See `sx0`.
    pub sh: u32,
    /// The destination rectangle in the frame (already clipped to it): origin and size. The source
    /// is scaled onto it by nearest-neighbour sampling.
    pub ox: u32,
    /// See `ox`.
    pub oy: u32,
    /// See `ox`.
    pub dw: u32,
    /// See `ox`.
    pub dh: u32,
    /// log2 chroma subsampling (x, y).
    pub sub_x_log2: u32,
    /// See `sub_x_log2`.
    pub sub_y_log2: u32,
    /// The chroma pair is V then U (after `SWAP_UV`).
    pub vu_first: bool,
}

/// ★ Plan semi-planar YUV window `s` of a `fw` x `fh` composition. `y_dma` is the luma plane's
/// context DMA, `c_dma` the chroma plane's (the same when the window names one). Every byte either
/// plane's kernel can read is bounded against its context DMA here; the destination is clipped to
/// the frame. `Ok(None)` for a window wholly outside the frame.
///
/// ⊘ What the console does NOT do (stated, not hidden): it ignores the window's colour pipeline for
/// a YUV window and converts BT.709 limited range; it scales by nearest neighbour; the window is
/// composed opaque, after the RGB windows. It is the console's view only; the guest's flips never
/// depend on it.
///
/// # Errors
/// [`Refused`], naming the bound: a system-memory surface, a rectangle outside the surface, a
/// block-linear geometry the kernel cannot walk, a pitch too small, any byte outside a context DMA.
pub fn plan_yuv(
    s: &Scanout,
    f: YuvFormat,
    y_dma: &CtxDma,
    c_dma: &CtxDma,
    fw: u32,
    fh: u32,
) -> Result<Option<YuvPlan>, Refused> {
    let refuse = |why: String| Refused(format!("window {} head {}: YUV: {why}", s.window, s.head));
    let no = |why: String| Err(refuse(why));
    if y_dma.target != Target::Vidmem || c_dma.target != Target::Vidmem {
        return no("a system-memory surface (the console shows video memory only)".into());
    }
    if y_dma.block_linear != c_dma.block_linear {
        return no("the planes disagree on pitch/block-linear".into());
    }
    if s.width == 0 || s.height == 0 || s.surface_width == 0 || s.surface_height == 0 {
        return no(format!(
            "an empty source rectangle {}x{}",
            s.width, s.height
        ));
    }
    if u64::from(s.width) * u64::from(s.height) > MAX_PIXELS {
        return no(format!(
            "{}x{} is larger than the console's pixels",
            s.width, s.height
        ));
    }
    if u64::from(s.x) + u64::from(s.width) > u64::from(s.surface_width)
        || u64::from(s.y) + u64::from(s.height) > u64::from(s.surface_height)
    {
        return no(format!(
            "rectangle {}x{}+{}+{} leaves the {}x{} surface",
            s.width, s.height, s.x, s.y, s.surface_width, s.surface_height
        ));
    }
    if s.out_x >= fw || s.out_y >= fh {
        return Ok(None);
    }
    // the destination: SET_SIZE_OUT (0 = unscaled), clipped to the frame
    let dw = if s.out_width == 0 {
        s.width
    } else {
        s.out_width
    };
    let dh = if s.out_height == 0 {
        s.height
    } else {
        s.out_height
    };
    let (dw, dh) = (dw.min(fw - s.out_x), dh.min(fh - s.out_y));
    if dw == 0 || dh == 0 {
        return Ok(None);
    }
    // the last source column/row the kernel can touch (nearest sampling stays inside the rectangle)
    let x_end = u64::from(s.x) + u64::from(s.width); // luma, exclusive
    let y_end = u64::from(s.y) + u64::from(s.height);
    let cx_end = x_end.div_ceil(1 << f.sub_x_log2); // chroma pairs, exclusive
    let cy_end = y_end.div_ceil(1 << f.sub_y_log2);
    let bound = |plane: &str,
                 dma: &CtxDma,
                 base: u64,
                 pitch_units: u32,
                 bytes_per_px: u64,
                 x_end: u64,
                 y_end: u64|
     -> Result<(u64, u64, u32), String> {
        if dma.block_linear {
            let bh = s.block_height_log2;
            if bh > 5 {
                return Err(format!(
                    "{plane}: SET_STORAGE.BLOCK_HEIGHT {bh} is not 1..32 GOBs"
                ));
            }
            let gobs = u64::from(pitch_units);
            if gobs == 0 || x_end * bytes_per_px > gobs * 64 {
                return Err(format!(
                    "{plane}: {} bytes leave the {gobs}-GOB-wide surface",
                    x_end * bytes_per_px
                ));
            }
            let block_rows = y_end.div_ceil(8u64 << bh);
            let extent = block_rows * gobs * (GOB_BYTES << bh);
            let src = dma.span(base, extent).ok_or_else(|| {
                format!(
                    "{plane}: [{base:#x}, +{extent:#x}) (block-linear) leaves context DMA {:#x}..={:#x}",
                    dma.base, dma.limit
                )
            })?;
            Ok((src, extent, pitch_units))
        } else {
            let pitch = u64::from(pitch_units) * 64;
            if x_end * bytes_per_px > pitch {
                return Err(format!(
                    "{plane}: a {}-byte row is wider than the {pitch}-byte pitch",
                    x_end * bytes_per_px
                ));
            }
            let extent = (y_end - 1) * pitch + x_end * bytes_per_px;
            let src = dma.span(base, extent).ok_or_else(|| {
                format!(
                    "{plane}: [{base:#x}, +{extent:#x}) leaves context DMA {:#x}..={:#x}",
                    dma.base, dma.limit
                )
            })?;
            Ok((
                src,
                extent,
                u32::try_from(pitch).map_err(|_| "pitch".to_string())?,
            ))
        }
    };
    let (y_src, y_extent, y_pitch) =
        bound("luma", y_dma, s.offset, s.pitch, 1, x_end, y_end).map_err(refuse)?;
    // ⊘ [measured, run 401, Windows 11 / Edge MPO overlay, 2026-10-10] the guest programs
    // `SET_PLANAR_STORAGE(1)` but leaves `SET_CONTEXT_DMA_ISO(1)` and `SET_OFFSET(1)` at 0: no
    // separate chroma allocation. The console then read the chroma plane from the luma plane's
    // first byte (magenta/green picture). A chroma plane that was not programmed follows the luma
    // plane in the same surface: its first byte is the end of the luma plane's whole surface.
    let chroma_off = if s.iso1 == 0 && s.offset1 == 0 {
        let rows = u64::from(s.surface_height);
        let luma_bytes = if y_dma.block_linear {
            rows.div_ceil(8u64 << s.block_height_log2)
                * u64::from(s.pitch)
                * (GOB_BYTES << s.block_height_log2)
        } else {
            rows * u64::from(s.pitch) * 64
        };
        s.offset.saturating_add(luma_bytes.next_multiple_of(256))
    } else {
        s.offset1
    };
    let (c_src, c_extent, c_pitch) =
        bound("chroma", c_dma, chroma_off, s.pitch1, 2, cx_end, cy_end).map_err(refuse)?;
    Ok(Some(YuvPlan {
        window: s.window,
        y_src,
        y_extent,
        c_src,
        c_extent,
        block_linear: y_dma.block_linear,
        y_pitch,
        c_pitch,
        block_height_log2: s.block_height_log2,
        sx0: s.x,
        sy0: s.y,
        sw: s.width,
        sh: s.height,
        ox: s.out_x,
        oy: s.out_y,
        dw,
        dh,
        sub_x_log2: f.sub_x_log2,
        sub_y_log2: f.sub_y_log2,
        vu_first: f.vu_first != s.swap_uv,
    }))
}

/// ★ The YUV -> RGB arithmetic of the kernel (`kf_yuv.cu`), BT.709 limited range, 8.8 fixed point,
/// for one pixel: `0xFFRRGGBB`.
#[must_use]
pub fn yuv_to_xrgb(y: u8, u: u8, v: u8) -> u32 {
    let c = 298 * (i32::from(y) - 16);
    let (d, e) = (i32::from(u) - 128, i32::from(v) - 128);
    let q = |x: i32| ((x + 128) >> 8).clamp(0, 255) as u32;
    let r = q(c + 459 * e);
    let g = q(c - 55 * d - 136 * e);
    let b = q(c + 541 * d);
    0xFF00_0000 | (r << 16) | (g << 8) | b
}

/// ★ The kernel's address arithmetic and sampling on the CPU: the REFERENCE for `kf_compose_yuv`.
/// `ysrc`/`csrc` hold the store bytes from [`YuvPlan::y_src`]/[`YuvPlan::c_src`]; `frame` is the
/// `fw` x `fh` XRGB staging frame, composed into in place (opaque).
///
/// # Errors
/// A read outside a plane's extent, or a short frame.
pub fn yuv_reference(
    l: &YuvPlan,
    ysrc: &[u8],
    csrc: &[u8],
    frame: &mut [u8],
    fw: u32,
    fh: u32,
) -> Result<(), Refused> {
    if (frame.len() as u64) < u64::from(fw) * u64::from(fh) * 4 {
        return Err(Refused("a short frame".into()));
    }
    let addr = |x_bytes: u64, y: u64, pitch: u32| -> u64 {
        if l.block_linear {
            bl_offset(x_bytes, y, u64::from(pitch), l.block_height_log2)
        } else {
            y * u64::from(pitch) + x_bytes
        }
    };
    let get = |buf: &[u8], extent: u64, a: u64| -> Result<u8, Refused> {
        if a >= extent {
            return Err(Refused(format!(
                "a read at {a:#x} past the plane's {extent:#x}"
            )));
        }
        buf.get(a as usize)
            .copied()
            .ok_or_else(|| Refused("a read past the supplied bytes".into()))
    };
    for row in 0..l.dh {
        let (dy, ox_row) = (l.oy + row, l.ox);
        if dy >= fh {
            continue;
        }
        let sy = u64::from(l.sy0) + u64::from(row) * u64::from(l.sh) / u64::from(l.dh);
        for x in 0..l.dw {
            let dx = ox_row + x;
            if dx >= fw {
                continue;
            }
            let sx = u64::from(l.sx0) + u64::from(x) * u64::from(l.sw) / u64::from(l.dw);
            let y = get(ysrc, l.y_extent, addr(sx, sy, l.y_pitch))?;
            let (cx, cy) = (sx >> l.sub_x_log2, sy >> l.sub_y_log2);
            let c0 = get(csrc, l.c_extent, addr(cx * 2, cy, l.c_pitch))?;
            let c1 = get(csrc, l.c_extent, addr(cx * 2 + 1, cy, l.c_pitch))?;
            let (u, v) = if l.vu_first { (c1, c0) } else { (c0, c1) };
            let p = (u64::from(dy) * u64::from(fw) + u64::from(dx)) as usize * 4;
            frame[p..p + 4].copy_from_slice(&yuv_to_xrgb(y, u, v).to_le_bytes());
        }
    }
    Ok(())
}

/// ★ The compose kernel's arithmetic on the CPU — the REFERENCE `cuda/display/kf_scanout.ptx`
/// (`kf_compose`) must match byte for byte: `tests/compose_kernel.rs` runs the committed PTX
/// itself, instruction by instruction, over every (row, thread) of the launch and compares; the
/// display worker's bring-up self-test compares the real launch on the GPU. `src` holds the
/// store's bytes from [`LayerPlan::src`] (at most [`LayerPlan::extent`] of them are read); `frame`
/// is the `fw` x `fh` XRGB8888 staging frame, rows `fw * 4` bytes, composed into in place.
///
/// Per pixel, in the kernel's order: the source word (red and blue swapped with
/// [`COMPOSE_SWAP_RB`]); [`COMPOSE_OPAQUE`] stores it; [`COMPOSE_XOR`] XORs its colour into what
/// lies below; otherwise `out = (src * fs + below * fd + 127) / 255` per channel (at most 255,
/// the X byte 0), `fs = clamp(a_s + b_s * a / 255)` and `fd = clamp(a_d + b_d * a / 255)` in
/// 0..=255 with C's truncating division, `a` the source alpha (255 without [`COMPOSE_ALPHA`]).
/// Pixels past the frame's right or bottom edge are skipped, as the kernel skips them.
///
/// # Errors
/// A read outside `src` (the plan's extent is the kernel's whole bound), or a `frame` shorter than
/// `fw * fh * 4` bytes.
pub fn compose_reference(
    l: &LayerPlan,
    src: &[u8],
    frame: &mut [u8],
    fw: u32,
    fh: u32,
) -> Result<(), Refused> {
    let need = u64::from(fw) * u64::from(fh) * 4;
    if (frame.len() as u64) < need {
        return Err(Refused(format!(
            "a {}-byte frame for {fw}x{fh}",
            frame.len()
        )));
    }
    let clamp = |a: i32, b: i32, alpha: i32| {
        a.wrapping_add(b.wrapping_mul(alpha) / 255).clamp(0, 255) as u32
    };
    for row in 0..l.rows {
        let dy = l.oy.wrapping_add(row);
        if dy >= fh {
            break;
        }
        for x in 0..l.width {
            let dx = l.ox.wrapping_add(x);
            if dx >= fw {
                break;
            }
            let at = if l.block_linear {
                bl_offset(
                    u64::from(l.x0_bytes.wrapping_add(x * 4)),
                    u64::from(l.y0.wrapping_add(row)),
                    u64::from(l.pitch),
                    l.block_height_log2,
                )
            } else {
                u64::from(row) * u64::from(l.pitch) + u64::from(x) * 4
            };
            let Some(word) = usize::try_from(at)
                .ok()
                .and_then(|a| src.get(a..a.checked_add(4)?))
            else {
                return Err(Refused(format!(
                    "pixel ({x}, {row}) reads {at:#x}, past the {:#x} source bytes",
                    src.len()
                )));
            };
            let mut w = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
            if l.flags & COMPOSE_SWAP_RB != 0 {
                w = (w & 0xff00_ff00) | ((w >> 16) & 0xff) | ((w & 0xff) << 16);
            }
            let p = ((dy as usize) * (fw as usize) + dx as usize) * 4;
            let below = u32::from_le_bytes([frame[p], frame[p + 1], frame[p + 2], frame[p + 3]]);
            let out = if l.flags & COMPOSE_OPAQUE != 0 {
                w
            } else if l.flags & COMPOSE_XOR != 0 {
                below ^ (w & 0x00ff_ffff)
            } else {
                let a = if l.flags & COMPOSE_ALPHA != 0 {
                    (w >> 24) as i32
                } else {
                    255
                };
                let (fs, fd) = (clamp(l.a_s, l.b_s, a), clamp(l.a_d, l.b_d, a));
                let ch = |shift: u32| {
                    let (sc, dc) = ((w >> shift) & 0xff, (below >> shift) & 0xff);
                    ((sc * fs + dc * fd + 127) / 255).min(255) << shift
                };
                ch(0) | ch(8) | ch(16)
            };
            frame[p..p + 4].copy_from_slice(&out.to_le_bytes());
        }
    }
    Ok(())
}

/// The [`LayerPlan::window`] a cursor layer carries (logs only: the cursor is no window). One below
/// [`BOOT_WINDOW`] (the merge of 2026-10-03: both had taken `u32::MAX`), so a log tells them apart.
pub const CURSOR_LAYER: u32 = u32::MAX - 1;

/// ★ Display step 3d: plan head `c.head`'s cursor as the TOP layer of a `fw` x `fh` composition.
/// The image is pitch `A8R8G8B8` (NVKMS programs nothing else, `ogkm-580:
/// src/nvidia-modeset/src/nvkms-evo3.c:6512-6524`), square, with a pitch of `size * 4` but at least
/// 256 bytes (`:6531-6552`); placed with its hot spot at the cursor channel's point, clipped to the
/// frame on every side; blended by `HEAD_SET_CONTROL_CURSOR_COMPOSITION` with the window factor
/// numbering. `Ok(None)` when it lies wholly outside the frame.
///
/// ★ **`MODE_XOR`** (2026-10-04, `OWNER_RULINGS.md` §O: "the compose kernel gains an XOR blend") is
/// composed with [`COMPOSE_XOR`]: `out = below XOR colour`, alpha and the factors ignored. ⊘ This
/// is a DEFINITION, not a measurement, and here is why it had to be one: NVKMS never programs it.
/// Its cursor composition table (`ogkm-580: src/nvidia-modeset/src/nvkms-evo3.c:6646-6702`, the
/// same five cases at `nvkms-evo4.c:965-1019`, and EVO2's two at `nvkms-evo2.c:3194-3206`) writes
/// `MODE_BLEND` (EVO2: `_ALPHA_BLEND`/`_PREMULT_ALPHA_BLEND`) for
/// all five blending modes it supports (`NV_EVO3_SUPPORTED_CURSOR_COMP_BLEND_MODES`,
/// `nvkms-evo3.h:44-49`: opaque, premultiplied and straight alpha, each with or without a surface
/// alpha), and its only cursor format is `A8R8G8B8` (`nvkms-evo3.c:6517-6524`); no factor pair can
/// invert either (`out = src*fs + dst*fd` with both factors in 0..=1). So an XOR or inverting
/// cursor reaches the head only from a guest that writes the core channel itself — a hostile one,
/// or another OS's driver — and the class header names the mode without saying what it computes.
/// The literal reading is taken: the colour is XORed into what lies below, so an all-zero image
/// composes to nothing and a white pixel inverts. ⚠ The masked-colour reading (alpha as the AND
/// mask, so one surface can carry opaque pixels too — what a Windows monochrome pointer needs) is
/// the alternative; it can be settled only on hardware, by a producer that programs the mode.
/// `A1R5G5B5` (`clc37d.h:836`), the class's other cursor format and the classic AND/XOR one, stays
/// refused: the kernel moves 32-bit pixels.
///
/// # Errors
/// [`Refused`], naming the bound: a non-`A8R8G8B8` format, an unknown composition mode, a
/// system-memory or block-linear context DMA, an unknown factor (blend only), or any byte outside
/// the context DMA.
pub fn plan_cursor(
    c: &CursorScan,
    dma: &CtxDma,
    fw: u32,
    fh: u32,
) -> Result<Option<LayerPlan>, Refused> {
    let no = |why: String| Err(Refused(format!("cursor head {}: {why}", c.head)));
    if !c.argb8888 {
        return no("only an A8R8G8B8 cursor is composable".into());
    }
    let xor = match c.mode {
        0 => false,
        1 => true,
        m => return no(format!("cursor composition mode {m} is not known")),
    };
    if dma.target != Target::Vidmem || dma.block_linear {
        return no("the cursor surface is not pitch video memory".into());
    }
    let size = c.size;
    let pitch = (size * 4).max(256);
    let left = i64::from(c.x) - i64::from(c.hot_x);
    let top = i64::from(c.y) - i64::from(c.hot_y);
    // the part of the image left of / above the frame is clipped away
    let (cx0, cy0) = ((-left).max(0), (-top).max(0));
    let (ox, oy) = (left.max(0), top.max(0));
    if cx0 >= i64::from(size)
        || cy0 >= i64::from(size)
        || ox >= i64::from(fw)
        || oy >= i64::from(fh)
    {
        return Ok(None);
    }
    let width = (i64::from(size) - cx0).min(i64::from(fw) - ox);
    let rows = (i64::from(size) - cy0).min(i64::from(fh) - oy);
    let as_u32 =
        |v: i64| u32::try_from(v).map_err(|_| Refused(format!("cursor head {}: {v}", c.head)));
    let (cx0, cy0, ox, oy, width, rows) = (
        as_u32(cx0)?,
        as_u32(cy0)?,
        as_u32(ox)?,
        as_u32(oy)?,
        as_u32(width)?,
        as_u32(rows)?,
    );
    let first = c.offset + u64::from(cy0) * u64::from(pitch) + u64::from(cx0) * 4;
    let extent = u64::from(rows - 1) * u64::from(pitch) + u64::from(width) * 4;
    let Some(src) = dma.span(first, extent) else {
        return no(format!(
            "[{first:#x}, +{extent:#x}) leaves context DMA {:#x}..={:#x}",
            dma.base, dma.limit
        ));
    };
    let (flags, ((a_s, b_s), (a_d, b_d))) = if xor {
        (COMPOSE_XOR, ((0, 0), (0, 0)))
    } else {
        let Some(f) = cursor_blend(c) else {
            return no(format!(
                "composition factors {:#x}/{:#x} are not known",
                c.cursor_factor, c.viewport_factor
            ));
        };
        let opaque = f == ((255, 0), (0, 0));
        // A8R8G8B8 carries alpha; no red/blue swap
        (COMPOSE_ALPHA | if opaque { COMPOSE_OPAQUE } else { 0 }, f)
    };
    Ok(Some(LayerPlan {
        window: CURSOR_LAYER,
        src,
        extent,
        block_linear: false,
        pitch,
        block_height_log2: 0,
        x0_bytes: 0,
        y0: 0,
        width,
        rows,
        ox,
        oy,
        flags,
        a_s,
        b_s,
        a_d,
        b_d,
    }))
}

/// The cursor's blend as `((a_s, b_s), (a_d, b_d))` in the compose kernel's factor numbering
/// ([`factor`]); `None` for an unknown selector.
fn cursor_blend(c: &CursorScan) -> Option<((i32, i32), (i32, i32))> {
    let k1 = i32::try_from(c.k1.min(255)).unwrap_or(255);
    // ⊘ a composition word never programmed (all zero) would erase the cursor; NVKMS programs it
    // with every image (`nvkms-evo3.c:6646-6700`), so zero is read as premultiplied alpha
    let (k1, cur_sel, vp_sel) = if c.k1 == 0 && c.cursor_factor == 0 && c.viewport_factor == 0 {
        (255, 2, 7)
    } else {
        (k1, c.cursor_factor, c.viewport_factor)
    };
    Some((factor(cur_sel, k1, 0)?, factor(vp_sel, k1, 0)?))
}

/// ★★ **The host cursor** (`OWNER_RULINGS.md` §O, hover mode; `docs/design/V3_DISPLAY.md` §8.12):
/// head `c.head`'s WHOLE cursor image as the host pointer's — the store span to copy (never clipped
/// to a frame: the host pointer may be anywhere) and the blend that turns its pixels into
/// premultiplied `ARGB8888` ([`HostCursorSrc::image`]). The GPU copies the span into a buffer the
/// VMM owns (`kf_cuda::display::DisplayGpu::read_store`, the path that already reads the display's
/// instance memory and pushbuffers); the CPU reads only that copy, never guest video memory
/// (`THE_CONSTRAINTS.md` §38).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostCursorSrc {
    /// The store offset of the image's first byte.
    pub src: u64,
    /// Bytes to copy: `size` rows `pitch` apart.
    pub extent: u64,
    /// The image's edge (32, 64, 128 or 256; square).
    pub size: u32,
    /// Bytes from one row to the next (`size * 4`, at least 256).
    pub pitch: u32,
    /// The hot spot, inside the image.
    pub hot: (u32, u32),
    /// The cursor's source factor `(a, b)`: `a + b * alpha / 255`, in 0..=255.
    src_factor: (i32, i32),
    /// The viewport's (destination) factor, likewise.
    dst_factor: (i32, i32),
}

/// ★ Plan head `c.head`'s cursor for the host ([`HostCursorSrc`]). A cursor the host cannot show
/// as premultiplied ARGB is refused by name — the caller then composes it into the frame in every
/// mode and hides the host's (§O). Some of that is decided by content
/// ([`HostCursorSrc::image`]).
///
/// # Errors
/// [`Refused`], naming why: a non-`A8R8G8B8` format; `XOR` composition (no ARGB "over" expresses
/// it); a system-memory or block-linear surface; a hot spot outside the image; an unknown factor;
/// any byte outside the context DMA.
pub fn plan_host_cursor(c: &CursorScan, dma: &CtxDma) -> Result<HostCursorSrc, Refused> {
    let no = |why: String| Err(Refused(format!("host cursor head {}: {why}", c.head)));
    if !c.argb8888 {
        return no("only an A8R8G8B8 cursor can be the host's".into());
    }
    if c.mode != 0 {
        return no("an XOR cursor has no ARGB equivalent".into());
    }
    if dma.target != Target::Vidmem || dma.block_linear {
        return no("the cursor surface is not pitch video memory".into());
    }
    if c.hot_x >= c.size || c.hot_y >= c.size {
        return no(format!(
            "hot spot {},{} outside the {}x{} image",
            c.hot_x, c.hot_y, c.size, c.size
        ));
    }
    let Some((src_factor, dst_factor)) = cursor_blend(c) else {
        return no(format!(
            "composition factors {:#x}/{:#x} are not known",
            c.cursor_factor, c.viewport_factor
        ));
    };
    // every selector of `factor` with K1 <= 255 and K2 = 0 gives a factor in 0..=255 at alpha 0
    // and at 255 (it is linear in alpha); `image`'s arithmetic relies on it, so it is checked
    let in_range = |(a, b): (i32, i32)| (0..=255).contains(&a) && (0..=255).contains(&(a + b));
    if !in_range(src_factor) || !in_range(dst_factor) {
        return no(format!(
            "blend factors {src_factor:?}/{dst_factor:?} leave 0..=1"
        ));
    }
    let size = c.size;
    let pitch = (size * 4).max(256);
    let extent = u64::from(size - 1) * u64::from(pitch) + u64::from(size) * 4;
    let Some(src) = dma.span(c.offset, extent) else {
        return no(format!(
            "[{:#x}, +{extent:#x}) leaves context DMA {:#x}..={:#x}",
            c.offset, dma.base, dma.limit
        ));
    };
    Ok(HostCursorSrc {
        src,
        extent,
        size,
        pitch,
        hot: (c.hot_x, c.hot_y),
        src_factor,
        dst_factor,
    })
}

/// ★ The cursor's `HEAD_SET_CONTROL_CURSOR_COMPOSITION` word as programmed (`K1` 7:0,
/// `CURSOR_COLOR_FACTOR_SELECT` 11:8, `VIEWPORT_COLOR_FACTOR_SELECT` 15:12, `MODE` 16:16,
/// `ogkm-580: clc37d.h:850-861`), and the NVKMS blending mode it spells
/// (`ogkm-580: src/nvidia-modeset/src/nvkms-evo3.c:6649-6701`) — for the log line that settles which
/// blend a guest programs over which pixels (§8.12: on the box the host's semi-transparent cursor
/// pixels were the guest's times alpha, which a straight-alpha blend over premultiplied pixels
/// would produce; the word was not logged then).
#[must_use]
pub fn cursor_composition(c: &CursorScan) -> (u32, &'static str) {
    let word = (c.k1 & 0xff)
        | (c.cursor_factor & 0xf) << 8
        | (c.viewport_factor & 0xf) << 12
        | (c.mode & 1) << 16;
    let name = match (c.mode, c.cursor_factor, c.viewport_factor, c.k1) {
        (1, ..) => "XOR (no NVKMS mode)",
        (0, 0, 0, 0) => "never programmed (composed as PREMULT_ALPHA)",
        (0, 2, 0, 255) => "OPAQUE",
        (0, 2, 7, 255) => "PREMULT_ALPHA",
        (0, 5, 7, 255) => "NON_PREMULT_ALPHA (straight alpha)",
        (0, 2, 7, _) => "PREMULT_SURFACE_ALPHA (K1 = the surface alpha)",
        (0, 5, 7, _) => "NON_PREMULT_SURFACE_ALPHA (straight alpha, K1 = the surface alpha)",
        _ => "no NVKMS mode",
    };
    (word, name)
}

/// ★ §8.14's composition line, BOUNDED: [`CompositionLog::changed`] counts every change of the word
/// and says when its line is due — the first [`CompositionLog::LINES`] changes, then every
/// [`CompositionLog::EVERY`]th. ⊘ The review of 2026-10-04: the line was "once per change" of a word
/// the GUEST programs, so a guest alternating two words wrote a line per frame (60-144 a second,
/// gigabytes a day of the VMM's stderr) where every neighbouring line is rate-limited.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompositionLog {
    last: Option<u32>,
    changes: u64,
}

impl CompositionLog {
    /// The first this many changes are logged …
    pub const LINES: u64 = 8;
    /// … then every this-many-th.
    pub const EVERY: u64 = 256;

    /// The cursor read saw `word`: `Some(n)` — the change's number — when the line is due; an
    /// unchanged word is silent and uncounted.
    pub fn changed(&mut self, word: u32) -> Option<u64> {
        if self.last == Some(word) {
            return None;
        }
        self.last = Some(word);
        self.changes += 1;
        let n = self.changes;
        (n <= Self::LINES || n.is_multiple_of(Self::EVERY)).then_some(n)
    }

    /// Changes seen so far.
    #[must_use]
    pub fn changes(&self) -> u64 {
        self.changes
    }
}

/// ★ What a cursor image's pixels say about their own alpha convention: how many are partially
/// transparent, and how many carry a colour channel ABOVE their alpha — impossible in a
/// premultiplied image, so 0 of those with some partial pixels is consistent with premultiplied
/// pixels (an X server's cursor), and any is proof of straight ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AlphaCensus {
    /// Pixels with 0 < alpha < 255.
    pub partial: u32,
    /// Pixels with a colour channel above their alpha.
    pub above_alpha: u32,
}

/// ★★ §O, found on hardware (box 54032077, run `brkA`, 2026-10-03): **NVKMS hard-codes the
/// hardware hot spot to 0** (`ogkm-580: src/nvidia-modeset/src/nvkms-evo3.c:6565-6569`, "Hard code
/// the cursor hotspot") and moves the image's top-left instead, so the hot spot the guest MEANT is
/// in no register: the first SET of that run said `hot 0,0` for an arrow whose X hot spot is not
/// there. It is where the guest's POINTER is, minus where the image's top-left is (`point -
/// programmed hot`). In hover the VMM knows that pointer — it injected it: `abs` in the broker's
/// `range`, which QEMU scales to the tablet's axis and the guest maps onto the head's `frame` — so
/// the hot spot is derived from it. `None` when the result lies outside the image (the pointer and
/// the cursor point belong to different moments, or the pointer is not over this head).
///
/// ⊘ CORRECTED the same day (run `brkA4`, kf3 `18562ba4`): one scaling (`abs * frame / range`)
/// derived `4,2` for an arrow the guest's X server holds at `3,1`, and `12,12` for its `11,11`
/// crosshair — one pixel off on both axes, every time. The pointer goes through TWO integer
/// scalings, each truncating: QEMU's onto the tablet's axis (`qemu_input_scale_axis`, QEMU 10.2.4
/// `ui/input.c:470-481`, `v = abs * 0x7fff / range` — kf3.c passes the broker's range as the
/// maximum) and the guest's back onto the head (libinput's `(v - min) * size / (max - min + 1)`,
/// truncated: brkA4's injected 48 of 1024 reached the guest as 47, its 8 of 695 as 7). Both are
/// modelled here.
#[must_use]
pub fn hot_from_pointer(
    c: &CursorScan,
    abs: (i32, i32),
    range: (u32, u32),
    frame: (u32, u32),
) -> Option<(u32, u32)> {
    if range.0 == 0 || range.1 == 0 {
        return None;
    }
    // QEMU's INPUT_EVENT_ABS_MAX: the tablet's axis is 0..=0x7fff
    const TABLET_MAX: i64 = 0x7fff;
    let guest = |a: i32, r: u32, f: u32| {
        (i64::from(a) * TABLET_MAX / i64::from(r)) * i64::from(f) / (TABLET_MAX + 1)
    };
    let gx = guest(abs.0, range.0, frame.0);
    let gy = guest(abs.1, range.1, frame.1);
    let hx = gx - (i64::from(c.x) - i64::from(c.hot_x));
    let hy = gy - (i64::from(c.y) - i64::from(c.hot_y));
    let s = i64::from(c.size);
    ((0..s).contains(&hx) && (0..s).contains(&hy))
        .then(|| (u32::try_from(hx).ok(), u32::try_from(hy).ok()))
        .and_then(|(x, y)| Some((x?, y?)))
}

impl HostCursorSrc {
    /// ★ The [`AlphaCensus`] of the image `raw` (the [`Self::extent`] bytes copied from
    /// [`Self::src`]); `None` when `raw` is not that extent.
    #[must_use]
    pub fn alpha_census(&self, raw: &[u8]) -> Option<AlphaCensus> {
        if raw.len() as u64 != self.extent {
            return None;
        }
        let (n, pitch) = (self.size as usize, self.pitch as usize);
        let mut c = AlphaCensus::default();
        for y in 0..n {
            let row = raw.get(y * pitch..y * pitch + n * 4)?;
            let (px, _) = row.as_chunks::<4>();
            for p in px {
                c.partial += u32::from(p[3] != 0 && p[3] != 255);
                c.above_alpha += u32::from(p[..3].iter().any(|ch| *ch > p[3]));
            }
        }
        Some(c)
    }

    /// ★ The image as the host takes it: `size` x `size` premultiplied `ARGB8888` (B, G, R, A
    /// bytes), rows tight — what the head's blend `out = src * fs(a) + dst * fd(a)` means as an
    /// "over" of premultiplied pixels: coverage `A' = 1 - fd(a)`, colour `P' = c * fs(a)`. Every
    /// blend NVKMS programs maps exactly (opaque, premultiplied, straight, and both with a surface
    /// alpha in K1). `Ok(None)` for an image wholly transparent: the guest shows no cursor.
    /// `raw` is the [`Self::extent`] bytes the GPU copied from [`Self::src`].
    ///
    /// # Errors
    /// [`Refused`]: `raw` is not the extent; or a pixel's colour exceeds its coverage by more than
    /// rounding (an additive blend, which a premultiplied "over" cannot express).
    pub fn image(&self, raw: &[u8]) -> Result<Option<Vec<u8>>, Refused> {
        if raw.len() as u64 != self.extent {
            return Err(Refused(format!(
                "host cursor: {} bytes copied for a {:#x}-byte image",
                raw.len(),
                self.extent
            )));
        }
        let n = self.size as usize;
        let pitch = self.pitch as usize;
        let ((a_s, b_s), (a_d, b_d)) = (self.src_factor, self.dst_factor);
        // round(v / d) for v >= 0
        let div = |v: i64, d: i64| (v + d / 2) / d;
        let mut out = vec![0u8; n * n * 4];
        let mut any = false;
        for y in 0..n {
            for x in 0..n {
                let p = &raw[y * pitch + x * 4..y * pitch + x * 4 + 4];
                let a = i64::from(p[3]);
                // both in 0..=255*255 (the factors were checked in range at both ends)
                let fs = i64::from(a_s) * 255 + i64::from(b_s) * a;
                let fd = i64::from(a_d) * 255 + i64::from(b_d) * a;
                let cover = div(255 * 255 - fd, 255);
                let o = &mut out[(y * n + x) * 4..(y * n + x) * 4 + 4];
                for (k, &c) in p.iter().take(3).enumerate() {
                    let v = div(i64::from(c) * fs, 255 * 255);
                    if v > cover + 1 {
                        return Err(Refused(format!(
                            "host cursor: pixel {x},{y} has colour {v} over coverage {cover} \
                             (an additive blend, not an ARGB \"over\")"
                        )));
                    }
                    o[k] = u8::try_from(v.min(cover)).unwrap_or(u8::MAX);
                }
                o[3] = u8::try_from(cover).unwrap_or(u8::MAX);
                any |= o.iter().any(|b| *b != 0);
            }
        }
        Ok(any.then_some(out))
    }
}

/// ★ The boot framebuffer kf3's option ROM published (`docs/design/V3_DISPLAY.md` §4.11): a pitch
/// XRGB8888 surface at store offset 0 — what RM will call FB 0 — of `bytes` (G) store bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootSurface {
    /// Visible width in pixels.
    pub width: u32,
    /// Visible height in lines.
    pub height: u32,
    /// Bytes from one line to the next (a multiple of 256: NVKMS's console rule).
    pub pitch: u32,
    /// The framebuffer's store bytes, G: nothing past it is ever read.
    pub bytes: u64,
}

/// The [`LayerPlan::window`] of the boot layer: no guest window owns it.
pub const BOOT_WINDOW: u32 = u32::MAX;

/// ★★ **The boot layer**: what the console shows before the guest's driver arms a head — the
/// firmware's framebuffer, read by the GPU from store `[0, G)` (the CPU never reads it, §38).
/// One opaque pitch layer at the frame's origin, `width`×`height`, bounded by G.
///
/// The surface is VMM-AUTHORED (kf3 chose the mode and G when it packed its ROM), but it is checked
/// like any guest surface, so a bad geometry is a named refusal rather than a copy past G.
///
/// # Errors
/// [`Refused`], naming the bound: an empty or oversized mode, a pitch that is not a multiple of 256
/// or is shorter than a line, or lines that do not fit G.
pub fn boot_layer(s: &BootSurface) -> Result<LayerPlan, Refused> {
    let no = |why: String| Err(Refused(format!("boot framebuffer: {why}")));
    if s.width == 0 || s.height == 0 {
        return no(format!("an empty mode {}x{}", s.width, s.height));
    }
    if u64::from(s.width) * u64::from(s.height) > MAX_PIXELS {
        return no(format!(
            "{}x{} is larger than the console's {MAX_PIXELS} pixels",
            s.width, s.height
        ));
    }
    let line = u64::from(s.width) * 4;
    if !s.pitch.is_multiple_of(256) || u64::from(s.pitch) < line {
        return no(format!(
            "pitch {} is not a multiple of 256 at least {line} bytes long",
            s.pitch
        ));
    }
    let extent = u64::from(s.height - 1) * u64::from(s.pitch) + line;
    if u64::from(s.pitch) * u64::from(s.height) > s.bytes {
        return no(format!(
            "{} lines of {} bytes leave the {:#x}-byte framebuffer",
            s.height, s.pitch, s.bytes
        ));
    }
    Ok(LayerPlan {
        window: BOOT_WINDOW,
        src: 0,
        extent,
        block_linear: false,
        pitch: s.pitch,
        block_height_log2: 0,
        x0_bytes: 0,
        y0: 0,
        width: s.width,
        rows: s.height,
        ox: 0,
        oy: 0,
        // opaque XRGB8888 in B, G, R, X byte order: no alpha, no red/blue swap, store the word
        flags: 4,
        a_s: 255,
        b_s: 0,
        a_d: 0,
        b_d: 0,
    })
}

#[cfg(test)]
mod tests {
    /// ★ The composition line is bounded: a guest alternating two words 1000 times gets the first
    /// eight changes and every 256th logged, an unchanged word nothing. Known-positive: each of the
    /// first eight changes IS logged, numbered.
    #[test]
    fn the_composition_line_is_bounded_however_the_guest_alternates() {
        let mut l = CompositionLog::default();
        assert_eq!(l.changed(0x072ff), Some(1));
        assert_eq!(l.changed(0x072ff), None, "unchanged: silent");
        let mut lines = 1;
        for k in 0..1000u32 {
            if l.changed(if k % 2 == 0 { 0x075ff } else { 0x072ff })
                .is_some()
            {
                lines += 1;
            }
        }
        assert_eq!(l.changes(), 1001);
        assert_eq!(lines, 8 + 1001 / 256, "{lines} lines for 1001 changes");
        let mut l = CompositionLog::default();
        for n in 1..=8u64 {
            assert_eq!(l.changed(n as u32), Some(n));
        }
    }

    use super::*;

    /// 1920x1080 as kf3 packs it (`kf_oprom::Geometry::for_mode`): pitch 7680, G = 0x7F0000.
    fn boot1080() -> BootSurface {
        BootSurface {
            width: 1920,
            height: 1080,
            pitch: 7680,
            bytes: 0x7F_0000,
        }
    }

    #[test]
    fn the_boot_layer_is_one_opaque_pitch_copy_of_store_zero() {
        let l = boot_layer(&boot1080()).unwrap();
        assert_eq!((l.src, l.pitch, l.width, l.rows), (0, 7680, 1920, 1080));
        assert_eq!(l.extent, 1079 * 7680 + 1920 * 4);
        assert!(l.extent <= 0x7F_0000);
        assert!(!l.block_linear);
        assert_eq!((l.ox, l.oy, l.x0_bytes, l.y0), (0, 0, 0, 0));
        assert_eq!(l.flags, 4, "opaque, no alpha, no swap");
        assert_eq!((l.a_s, l.b_s, l.a_d, l.b_d), (255, 0, 0, 0));
        assert_eq!(l.window, BOOT_WINDOW);
        // the same shape `plan_layer` gives an opaque X8R8G8B8 pitch window at the origin
        let odd = BootSurface {
            width: 1152,
            height: 648,
            pitch: 4608,
            bytes: 0x2E_0000,
        };
        assert_eq!(boot_layer(&odd).unwrap().extent, 647 * 4608 + 1152 * 4);
    }

    #[test]
    fn a_boot_geometry_past_its_bounds_is_refused_by_name() {
        let b = boot1080();
        for (bad, word) in [
            (BootSurface { width: 0, ..b }, "empty"),
            (
                BootSurface {
                    width: 7680,
                    height: 4320,
                    pitch: 30720,
                    bytes: 1 << 30,
                },
                "larger",
            ),
            (BootSurface { pitch: 7700, ..b }, "pitch"),
            (BootSurface { pitch: 7424, ..b }, "pitch"),
            (
                BootSurface {
                    bytes: 0x7E_0000,
                    ..b
                },
                "leave",
            ),
        ] {
            let e = boot_layer(&bad).unwrap_err().0;
            assert!(
                e.contains(word) && e.contains("boot framebuffer"),
                "{bad:?}: {e}"
            );
        }
    }

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
            iso1: 0,
            offset1: 0,
            pitch1: 0,
            swap_uv: false,
            depth: 255,
            k1: 255,
            k2: 0,
            src_factor: 1,
            dst_factor: 0,
        }
    }

    /// ★ Edge's video overlay (window 4, `Y8___V8U8_N420`, 0x38): the format is derived from the
    /// class names, its planes are bounded one by one against their context DMAs, the chroma order
    /// follows `SWAP_UV`, and a plane that leaves its context DMA is refused by name.
    #[test]
    fn a_semi_planar_yuv_window_is_planned_plane_by_plane() {
        let f = formats();
        let yf = f
            .yuv_of(0x38)
            .expect("Y8___V8U8_N420 is in the class table");
        assert_eq!(
            (yf.sub_x_log2, yf.sub_y_log2, yf.vu_first),
            (1, 1, true),
            "NV21: V then U, 4:2:0"
        );
        assert!(f.of(0x38).is_none(), "not an RGB console format");
        assert!(f.yuv_of(0xE6).is_none());
        let s = Scanout {
            window: 4,
            head: 0,
            format: 0x38,
            width: 720,
            height: 1280,
            surface_width: 720,
            surface_height: 1280,
            pitch: 768 / 64,
            offset: 0x1000,
            iso1: 0x10099,
            offset1: 0x1000 + 768 * 1280,
            pitch1: 768 / 64,
            out_x: 22,
            out_y: 13,
            out_width: 1295,
            out_height: 986,
            ..fb1080()
        };
        let dma = vid(0x4000_0000, 64 << 20);
        let p = plan_yuv(&s, yf, &dma, &dma, 1920, 1080).unwrap().unwrap();
        assert_eq!((p.y_src, p.c_src), (0x4000_1000, 0x4000_1000 + 768 * 1280));
        assert_eq!(p.y_extent, 1279 * 768 + 720);
        assert_eq!(
            p.c_extent,
            639 * 768 + 720,
            "360 chroma pairs of 2 bytes, 640 rows"
        );
        assert_eq!((p.ox, p.oy, p.dw, p.dh), (22, 13, 1295, 986));
        assert!(p.vu_first);
        let swapped = Scanout { swap_uv: true, ..s };
        assert!(
            !plan_yuv(&swapped, yf, &dma, &dma, 1920, 1080)
                .unwrap()
                .unwrap()
                .vu_first
        );
        // the destination is clipped to the frame; a window past the frame is none
        let clipped = plan_yuv(&s, yf, &dma, &dma, 800, 600).unwrap().unwrap();
        assert_eq!((clipped.dw, clipped.dh), (800 - 22, 600 - 13));
        assert_eq!(
            plan_yuv(&Scanout { out_x: 1920, ..s }, yf, &dma, &dma, 1920, 1080),
            Ok(None)
        );
        // Windows' overlay (run 401): plane 1 never programmed (ISO(1) = OFFSET(1) = 0), block-linear
        // 448x796, 7 GOBs wide, 2 GOBs per block: the chroma plane follows the luma surface
        let win = Scanout {
            iso1: 0,
            offset1: 0,
            pitch: 7,
            pitch1: 7,
            width: 448,
            height: 796,
            surface_width: 448,
            surface_height: 796,
            block_height_log2: 1,
            offset: 0x10_0000,
            out_width: 448,
            out_height: 795,
            ..s
        };
        let blm = CtxDma {
            block_linear: true,
            ..dma
        };
        let p = plan_yuv(&win, yf, &blm, &blm, 1920, 1080).unwrap().unwrap();
        assert_eq!(p.y_extent, 50 * 7 * 1024);
        assert_eq!(
            p.c_src - p.y_src,
            358_400,
            "the chroma plane starts where the luma surface ends"
        );
        assert_eq!(p.c_extent, 25 * 7 * 1024);
        // hostile: the chroma plane leaves its context DMA by one byte; the luma plane does
        let small = vid(0x4000_0000, 0x1000 + 768 * 1280 + 639 * 768 + 719);
        let e = plan_yuv(&s, yf, &dma, &small, 1920, 1080).unwrap_err();
        assert!(
            e.0.contains("chroma") && e.0.contains("leaves context DMA"),
            "{e:?}"
        );
        let e = plan_yuv(
            &s,
            yf,
            &vid(0x4000_0000, 0x1000 + 1279 * 768 + 719),
            &dma,
            1920,
            1080,
        )
        .unwrap_err();
        assert!(e.0.contains("luma"), "{e:?}");
        // a chroma pitch narrower than the chroma row, a rectangle outside the surface, sysmem
        assert!(plan_yuv(&Scanout { pitch1: 1, ..s }, yf, &dma, &dma, 1920, 1080).is_err());
        assert!(plan_yuv(&Scanout { width: 721, ..s }, yf, &dma, &dma, 1920, 1080).is_err());
        let sys = CtxDma {
            target: Target::Sysmem,
            ..dma
        };
        assert!(plan_yuv(&s, yf, &sys, &dma, 1920, 1080).is_err());
        // block-linear: both planes, GOB columns and block rows bounded
        let bl = CtxDma {
            block_linear: true,
            ..dma
        };
        let sb = Scanout {
            pitch: 12,
            pitch1: 12,
            block_height_log2: 2,
            ..s
        };
        let p = plan_yuv(&sb, yf, &bl, &bl, 1920, 1080).unwrap().unwrap();
        assert_eq!(p.y_extent, 1280u64.div_ceil(32) * 12 * (512 << 2));
        assert_eq!(p.c_extent, 640u64.div_ceil(32) * 12 * (512 << 2));
        assert!(
            plan_yuv(
                &Scanout {
                    block_height_log2: 6,
                    ..sb
                },
                yf,
                &bl,
                &bl,
                1920,
                1080
            )
            .is_err()
        );
        assert!(plan_yuv(&Scanout { pitch: 11, ..sb }, yf, &bl, &bl, 1920, 1080).is_err());
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

    /// ★ The GOB swizzle, bit by bit (`x[3:0] y[1:0] x[4] y[2] x[5]`, measured in m3b, 2026-09-30,
    /// GA106 / 580.159.04), and GOBs stacked into blocks.
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

    fn cursor(x: i32, y: i32) -> CursorScan {
        CursorScan {
            head: 0,
            client: 0xc1d0_0015,
            handle: 0x2000,
            offset: 0x1000,
            size: 64,
            argb8888: true,
            hot_x: 0,
            hot_y: 0,
            x,
            y,
            k1: 255,
            cursor_factor: 2,
            viewport_factor: 7,
            mode: 0,
        }
    }

    /// ★ 3d: a 64x64 premultiplied cursor inside the frame is one pitch layer at its point, with
    /// the premultiplied blend (`src + dst * (1 - a)`), on top.
    #[test]
    fn a_cursor_inside_the_frame_is_a_premultiplied_pitch_layer_at_its_point() {
        let l = plan_cursor(&cursor(100, 50), &vid(0x4000_0000, 1 << 20), 1920, 1080)
            .unwrap()
            .unwrap();
        assert_eq!(l.window, CURSOR_LAYER);
        assert_eq!(
            (l.src, l.pitch, l.width, l.rows),
            (0x4000_1000, 256, 64, 64)
        );
        assert_eq!((l.ox, l.oy), (100, 50));
        assert_eq!((l.a_s, l.b_s, l.a_d, l.b_d), (255, 0, 255, -255));
        assert_eq!(l.flags, 1, "alpha, no swap, not opaque");
        assert_eq!(l.extent, 63 * 256 + 64 * 4);
    }

    /// The edges: hanging off the top-left (a negative point) clips the image's first rows and
    /// columns; off the bottom-right clips its last; wholly outside is no layer.
    #[test]
    fn a_cursor_is_clipped_on_every_edge() {
        let dma = vid(0x4000_0000, 1 << 20);
        let l = plan_cursor(&cursor(-10, -20), &dma, 1920, 1080)
            .unwrap()
            .unwrap();
        assert_eq!((l.ox, l.oy, l.width, l.rows), (0, 0, 54, 44));
        assert_eq!(
            l.src,
            0x4000_1000 + 20 * 256 + 10 * 4,
            "starts inside the image"
        );
        let l = plan_cursor(&cursor(1900, 1070), &dma, 1920, 1080)
            .unwrap()
            .unwrap();
        assert_eq!((l.ox, l.oy, l.width, l.rows), (1900, 1070, 20, 10));
        assert_eq!(plan_cursor(&cursor(-64, 0), &dma, 1920, 1080), Ok(None));
        assert_eq!(plan_cursor(&cursor(1920, 0), &dma, 1920, 1080), Ok(None));
        // a 32x32 cursor keeps the 256-byte minimum pitch
        let mut small = cursor(0, 0);
        small.size = 32;
        let l = plan_cursor(&small, &dma, 1920, 1080).unwrap().unwrap();
        assert_eq!((l.pitch, l.width), (256, 32));
    }

    /// ★ The composition word and its NVKMS name, for each of the five modes NVKMS programs
    /// (`nvkms-evo3.c:6649-6701`), XOR, an unprogrammed word and a foreign one; and the alpha census
    /// telling premultiplied-consistent pixels from straight ones.
    #[test]
    fn the_composition_word_names_its_nvkms_mode_and_the_census_reads_the_pixels() {
        let mut c = cursor(0, 0);
        for (k1, cf, vf, mode, word, name) in [
            (255, 2, 0, 0, 0x0_02ff, "OPAQUE"),
            (255, 2, 7, 0, 0x0_72ff, "PREMULT_ALPHA"),
            (255, 5, 7, 0, 0x0_75ff, "NON_PREMULT_ALPHA (straight alpha)"),
            (
                128,
                2,
                7,
                0,
                0x0_7280,
                "PREMULT_SURFACE_ALPHA (K1 = the surface alpha)",
            ),
            (
                128,
                5,
                7,
                0,
                0x0_7580,
                "NON_PREMULT_SURFACE_ALPHA (straight alpha, K1 = the surface alpha)",
            ),
            (255, 2, 7, 1, 0x1_72ff, "XOR (no NVKMS mode)"),
            (
                0,
                0,
                0,
                0,
                0,
                "never programmed (composed as PREMULT_ALPHA)",
            ),
            (255, 3, 7, 0, 0x0_73ff, "no NVKMS mode"),
        ] {
            (c.k1, c.cursor_factor, c.viewport_factor, c.mode) = (k1, cf, vf, mode);
            assert_eq!(cursor_composition(&c), (word, name));
        }
        let h = plan_host_cursor(&cursor(0, 0), &vid(0x4000_0000, 1 << 20)).unwrap();
        let mut raw = vec![0u8; h.extent as usize];
        // premultiplied: (64, 64, 64, 128) and an opaque pixel
        raw[..8].copy_from_slice(&[64, 64, 64, 128, 10, 20, 30, 255]);
        assert_eq!(
            h.alpha_census(&raw),
            Some(AlphaCensus {
                partial: 1,
                above_alpha: 0
            })
        );
        raw[8..12].copy_from_slice(&[200, 0, 0, 100]); // straight: a channel above its alpha
        assert_eq!(h.alpha_census(&raw).unwrap().above_alpha, 1);
        assert_eq!(h.alpha_census(&raw[1..]), None);
    }

    /// ★ §O (2026-10-04): an XOR cursor is composed — one pitch layer with [`COMPOSE_XOR`] and no
    /// factors, whatever its factor word says (an unknown one included: XOR does not read it);
    /// an unknown MODE is refused by name.
    #[test]
    fn an_xor_cursor_is_an_xor_layer_and_an_unknown_mode_is_refused() {
        let dma = vid(0x4000_0000, 1 << 20);
        let mut x = cursor(100, 50);
        x.mode = 1;
        x.cursor_factor = 0xf; // unknown: irrelevant to XOR
        let l = plan_cursor(&x, &dma, 1920, 1080).unwrap().unwrap();
        assert_eq!(l.flags, COMPOSE_XOR);
        assert_eq!((l.a_s, l.b_s, l.a_d, l.b_d), (0, 0, 0, 0));
        assert_eq!(
            (l.src, l.ox, l.oy, l.width, l.rows),
            (0x4000_1000, 100, 50, 64, 64)
        );
        x.mode = 2;
        assert!(
            plan_cursor(&x, &dma, 1920, 1080)
                .unwrap_err()
                .0
                .contains("mode 2")
        );
    }

    /// ★ The reference's arithmetic on hand-worked pixels (the kernel is checked against the same
    /// function by `tests/compose_kernel.rs`): XOR takes the colour, never the alpha, and an
    /// all-zero XOR pixel leaves the frame as it was; a premultiplied blend at a = 0x80 over grey;
    /// an opaque layer stores the word; a read past the source is refused.
    #[test]
    fn the_reference_composes_xor_blend_and_opaque_pixels_as_worked_by_hand() {
        let one = |flags, f: (i32, i32, i32, i32)| LayerPlan {
            window: 0,
            src: 0,
            extent: 8,
            block_linear: false,
            pitch: 8,
            block_height_log2: 0,
            x0_bytes: 0,
            y0: 0,
            width: 2,
            rows: 1,
            ox: 0,
            oy: 0,
            flags,
            a_s: f.0,
            b_s: f.1,
            a_d: f.2,
            b_d: f.3,
        };
        // B G R A, twice: a white pixel with alpha 0, an all-zero one
        let src = [0xff, 0xff, 0xff, 0x00, 0, 0, 0, 0];
        let mut frame = [0x12, 0x34, 0x56, 0x00, 0x9a, 0xbc, 0xde, 0x00];
        compose_reference(&one(COMPOSE_XOR, (0, 0, 0, 0)), &src, &mut frame, 2, 1).unwrap();
        assert_eq!(frame, [0xed, 0xcb, 0xa9, 0x00, 0x9a, 0xbc, 0xde, 0x00]);
        // premultiplied (1, 1 - a): fs = 255, fd = 255 - 255 * 128 / 255 = 127
        let src = [0x40, 0x40, 0x40, 0x80, 0, 0, 0, 0xff];
        let mut frame = [0x80, 0x80, 0x80, 0x00, 0x80, 0x80, 0x80, 0x00];
        let pm = (255, 0, 255, -255);
        compose_reference(&one(COMPOSE_ALPHA, pm), &src, &mut frame, 2, 1).unwrap();
        assert_eq!(
            &frame[..3],
            &[128, 128, 128],
            "(64*255 + 128*127 + 127) / 255"
        );
        assert_eq!(&frame[4..7], &[0, 0, 0], "an opaque black pixel");
        let mut frame = [0u8; 8];
        compose_reference(&one(COMPOSE_OPAQUE, (255, 0, 0, 0)), &src, &mut frame, 2, 1).unwrap();
        assert_eq!(frame, src, "opaque: the word");
        assert!(
            compose_reference(
                &one(COMPOSE_OPAQUE, (255, 0, 0, 0)),
                &src[..7],
                &mut frame,
                2,
                1
            )
            .is_err()
        );
    }

    /// ⊘ Refused by name: a non-ARGB format, a sysmem surface, bytes past the context DMA.
    #[test]
    fn a_cursor_the_kernel_cannot_compose_is_refused_by_name() {
        let dma = vid(0x4000_0000, 1 << 20);
        let mut f = cursor(0, 0);
        f.argb8888 = false;
        assert!(
            plan_cursor(&f, &dma, 1920, 1080)
                .unwrap_err()
                .0
                .contains("A8R8G8B8")
        );
        let sys = CtxDma {
            target: Target::Sysmem,
            ..dma
        };
        assert!(plan_cursor(&cursor(0, 0), &sys, 1920, 1080).is_err());
        let tiny = vid(0x4000_0000, 0x2000);
        assert!(
            plan_cursor(&cursor(0, 0), &tiny, 1920, 1080)
                .unwrap_err()
                .0
                .contains("leaves context DMA")
        );
        // an unprogrammed composition word is premultiplied, never an eraser
        let mut z = cursor(0, 0);
        (z.k1, z.cursor_factor, z.viewport_factor) = (0, 0, 0);
        let l = plan_cursor(&z, &dma, 1920, 1080).unwrap().unwrap();
        assert_eq!((l.a_s, l.b_s, l.a_d, l.b_d), (255, 0, 255, -255));
        // NVKMS's opaque mode: K1 = 255, cursor K1, viewport ZERO
        let mut o = cursor(0, 0);
        o.viewport_factor = 0;
        let l = plan_cursor(&o, &dma, 1920, 1080).unwrap().unwrap();
        assert_eq!(l.flags, COMPOSE_ALPHA | COMPOSE_OPAQUE, "opaque");
    }
    // ── §O: the host cursor ─────────────────────────────────────────────────────────────────

    /// A 32x32 cursor with this blend, its plan, and a raw copy whose pixel (0, 0) is `px`
    /// (B, G, R, A) and every other pixel zero.
    fn host32(cur: u32, vp: u32, k1: u32, px: [u8; 4]) -> (HostCursorSrc, Vec<u8>) {
        let mut c = cursor(0, 0);
        (c.size, c.cursor_factor, c.viewport_factor, c.k1) = (32, cur, vp, k1);
        let h = plan_host_cursor(&c, &vid(0x4000_0000, 1 << 20)).unwrap();
        let mut raw = vec![0u8; h.extent as usize];
        raw[..4].copy_from_slice(&px);
        (h, raw)
    }

    /// ★ The host gets the WHOLE image wherever the guest's point is (the host pointer can be
    /// anywhere): never clipped, at the 256-byte minimum pitch, with the hot spot.
    #[test]
    fn the_host_cursor_is_the_whole_image_wherever_the_point_is() {
        let dma = vid(0x4000_0000, 1 << 20);
        let mut c = cursor(-500, 5000);
        (c.hot_x, c.hot_y) = (3, 7);
        let h = plan_host_cursor(&c, &dma).unwrap();
        assert_eq!(
            (h.src, h.size, h.pitch, h.hot),
            (0x4000_1000, 64, 256, (3, 7))
        );
        assert_eq!(h.extent, 63 * 256 + 256);
        let mut s = cursor(0, 0);
        s.size = 32;
        let h = plan_host_cursor(&s, &dma).unwrap();
        assert_eq!((h.pitch, h.extent), (256, 31 * 256 + 128));
    }

    /// ★ Every blend NVKMS programs for a cursor (`nvkms-evo3.c:6646-6700`) as premultiplied ARGB:
    /// premultiplied as is, straight alpha premultiplied, opaque as alpha 255, both surface-alpha
    /// forms scaled by K1, and the never-programmed word read as premultiplied. Expected values
    /// worked by hand (round half up), not by the formula under test.
    #[test]
    fn every_nvkms_cursor_blend_becomes_premultiplied_argb() {
        for (name, cur, vp, k1, px, want) in [
            ("PREMULT", 2, 7, 255, [100, 50, 25, 128], [100, 50, 25, 128]),
            (
                "NON_PREMULT",
                5,
                7,
                255,
                [200, 100, 50, 128],
                [100, 50, 25, 128],
            ),
            ("OPAQUE", 2, 0, 255, [10, 20, 30, 0], [10, 20, 30, 255]),
            (
                "PREMULT_SURFACE",
                2,
                7,
                128,
                [100, 50, 25, 128],
                [50, 25, 13, 64],
            ),
            (
                "NON_PREMULT_SURFACE",
                5,
                7,
                128,
                [200, 100, 50, 128],
                [50, 25, 13, 64],
            ),
            (
                "unprogrammed",
                0,
                0,
                0,
                [100, 50, 25, 128],
                [100, 50, 25, 128],
            ),
        ] {
            let (h, raw) = host32(cur, vp, k1, px);
            let out = h.image(&raw).unwrap().unwrap_or_else(|| panic!("{name}"));
            assert_eq!(out.len(), 32 * 32 * 4, "{name}: tight rows");
            assert_eq!(out[..4], want, "{name}");
            if name != "OPAQUE" {
                assert!(out[4..].iter().all(|b| *b == 0), "{name}: the rest");
            }
        }
        // the row pitch of the source is honoured: pixel (0, 1) is 256 bytes in, 128 out
        let (h, mut raw) = host32(2, 7, 255, [0; 4]);
        raw[256..260].copy_from_slice(&[9, 8, 7, 200]);
        let out = h.image(&raw).unwrap().unwrap();
        assert_eq!(out[128..132], [9, 8, 7, 200]);
    }

    /// ★ A wholly transparent image is NO cursor (the guest hides it that way too) — while the same
    /// zero bytes under the OPAQUE blend are an opaque black square, which is shown.
    #[test]
    fn a_wholly_transparent_cursor_is_no_host_cursor() {
        let (h, raw) = host32(2, 7, 255, [0; 4]);
        assert_eq!(h.image(&raw), Ok(None));
        let (h, raw) = host32(2, 0, 255, [0; 4]);
        let out = h.image(&raw).unwrap().expect("an opaque square is visible");
        assert!(out.chunks(4).all(|p| p == [0, 0, 0, 255]));
    }

    /// ★ NVKMS programs hot spot 0 and places the image's top-left at the point, so the hot spot
    /// is the pointer minus the point — the pointer as the GUEST computes it from what was
    /// injected (QEMU's tablet scaling, then the guest's, each truncating); outside the image is
    /// no answer.
    #[test]
    fn the_hot_spot_is_the_pointer_minus_the_images_top_left() {
        // [measured brkA4, 2026-10-03, RTX 3060 / 580.159.04] 48 of 1024 reached the guest as 47,
        // 8 of 695 as 7: a cursor whose top-left is at (44, 6) has its hot spot at (3, 1)
        let mut m = cursor(44, 6);
        (m.hot_x, m.hot_y) = (0, 0);
        assert_eq!(
            hot_from_pointer(&m, (48, 8), (1024, 695), (1024, 695)),
            Some((3, 1))
        );
        // NVKMS: hot 0, the image's top-left at (700, 400); injected (704, 406) is the guest's
        // (703, 405)
        let mut c = cursor(700, 400);
        (c.hot_x, c.hot_y) = (0, 0);
        assert_eq!(
            hot_from_pointer(&c, (704, 406), (1024, 768), (1024, 768)),
            Some((3, 5))
        );
        // the broker's range is not the head's size: scaled into the head's pixels
        assert_eq!(
            hot_from_pointer(&c, (1408, 812), (2048, 1536), (1024, 768)),
            Some((3, 5))
        );
        // a driver that programs its hot spot: the point IS the pointer, the result the same
        let mut p = cursor(703, 405);
        (p.hot_x, p.hot_y) = (3, 5);
        assert_eq!(
            hot_from_pointer(&p, (704, 406), (1024, 768), (1024, 768)),
            Some((3, 5))
        );
        // the pointer left of / below the image, a range of 0: no answer
        assert_eq!(
            hot_from_pointer(&c, (700, 406), (1024, 768), (1024, 768)),
            None
        );
        assert_eq!(
            hot_from_pointer(&c, (704, 465), (1024, 768), (1024, 768)),
            None
        );
        assert_eq!(
            hot_from_pointer(&c, (704, 406), (0, 768), (1024, 768)),
            None
        );
    }

    /// ⊘ What the host cannot show is refused by name — the caller composes it in every mode:
    /// XOR, a non-ARGB format, a sysmem or block-linear surface, a hot spot outside the image, an
    /// unknown factor, bytes past the context DMA, an additive blend, a short copy.
    #[test]
    fn a_cursor_the_host_cannot_show_is_refused_by_name() {
        let dma = vid(0x4000_0000, 1 << 20);
        let refused = |c: &CursorScan, d: &CtxDma, what: &str| {
            let e = plan_host_cursor(c, d).unwrap_err().0;
            assert!(e.contains(what), "{e}");
        };
        let mut x = cursor(0, 0);
        x.mode = 1;
        refused(&x, &dma, "XOR");
        let mut f = cursor(0, 0);
        f.argb8888 = false;
        refused(&f, &dma, "A8R8G8B8");
        let sys = CtxDma {
            target: Target::Sysmem,
            ..dma
        };
        refused(&cursor(0, 0), &sys, "pitch video memory");
        let bl = CtxDma {
            block_linear: true,
            ..dma
        };
        refused(&cursor(0, 0), &bl, "pitch video memory");
        let mut hot = cursor(0, 0);
        hot.hot_x = 64;
        refused(&hot, &dma, "hot spot");
        let mut u = cursor(0, 0);
        u.cursor_factor = 9;
        refused(&u, &dma, "not known");
        refused(
            &cursor(0, 0),
            &vid(0x4000_0000, 0x2000),
            "leaves context DMA",
        );
        // additive: viewport ONE keeps the whole destination — coverage 0 under a coloured pixel
        let (h, raw) = host32(2, 1, 255, [50, 0, 0, 0]);
        assert!(h.image(&raw).unwrap_err().0.contains("additive"));
        // ... and its transparent pixels are fine (no colour, nothing to cover)
        let (h, raw) = host32(2, 1, 255, [0; 4]);
        assert_eq!(h.image(&raw), Ok(None));
        let (h, raw) = host32(2, 7, 255, [0; 4]);
        assert!(h.image(&raw[1..]).unwrap_err().0.contains("bytes copied"));
    }
}
