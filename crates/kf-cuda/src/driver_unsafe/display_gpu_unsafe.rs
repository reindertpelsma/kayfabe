// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★★ **The display plane's GPU half — perimeter tier 2** (`v3-sec-rawaddr`, 2026-10-04; audit
//! S1-03, S1-04, S1-05(b)).
//!
//! The display plane's own CUDA context (a separate one from the walker's: the display worker must
//! never wait on a walk, nor a walk on a scanout copy): the imported store, the device staging
//! frame the windows compose into, the compose kernel, and the page-locked console frames QEMU
//! reads. The validation sites:
//!
//! - **V9** [`DisplayGpu::compose_begin`]: the composition's geometry, and the [`Composer`] that
//!   ties `(w, h)` to the staging buffer for as long as it lives;
//! - **V10** [`Composer::layer`] / [`compose_layer_fits`]: every read inside the layer's extent and
//!   the store, every write inside the composition;
//! - **V11** [`Composer::finish`]: the copy into a [`ConsoleFrame`] of this context that holds it;
//! - **V12** [`DisplayGpu::console_frame`]: the frame's size and the leak budget (F3).
//!
//! ⊘ No `unsafe`, no address. A console frame's memory is reachable from safe code only as the
//! opaque [`StaticSpan`] (`THE_CONSTRAINTS.md` §13), and the only thing the GPU copies into host
//! memory is a [`ConsoleFrame`].

use super::raw::{
    Arg, ConsoleDst, Ctx, DevMem, DevRange, Kernel, Module, Ptx, Stream, console_pages,
};
use super::{CompletionFd, CudaError, refused};
use crate::display::{COMPOSE_ENTRY, ComposeLayer};
use kf_linux_raw::{HostOffset, HostPageSize, MappedRegion, StaticSpan};
use std::os::fd::BorrowedFd;

/// ★ F3: console frames one [`DisplayGpu`] may mint (they are leaked for the process).
pub(crate) const MAX_CONSOLE_FRAMES: u32 = 16;
/// ★ F3: the largest console frame, in bytes (64 MiB; the largest mode is 3840×2160×4 ≈ 32 MiB).
pub(crate) const MAX_CONSOLE_FRAME_BYTES: u64 = 64 << 20;
/// ★ V9/V10: the largest composition, and the largest layer, in pixels per side.
pub(crate) const MAX_SIDE: u32 = 16384;

/// ★ The display worker's GPU context with the store imported.
pub struct DisplayGpu {
    ctx: Ctx,
    stream: Stream,
    done: CompletionFd,
    /// The compose kernel, or why it did not load (every scanout is refused by it).
    compose: Result<Kernel, String>,
    store: Option<DevMem>,
    /// The device staging frame the windows compose into.
    staging: Option<DevMem>,
    frames_minted: u32,
    frame_bytes_minted: u64,
}

impl core::fmt::Debug for DisplayGpu {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DisplayGpu")
            .field("device", &self.ctx.device_name())
            .field("store_bytes", &self.store.as_ref().map(DevMem::len))
            .field("frames_minted", &self.frames_minted)
            .finish_non_exhaustive()
    }
}

/// ★★ **A page-locked console frame** — host memory the GPU copies a finished composition into
/// and QEMU's console reads, zero-copy. Minted only by [`DisplayGpu::console_frame`]: registered
/// with the display context while still owned, then leaked, so it is **never unmapped** (QEMU may
/// read it after the surface moves on — screendump, the D-Bus listener).
///
/// Its memory leaves the perimeter only as [`ConsoleFrame::span`], an opaque [`StaticSpan`]. Not
/// `Copy`, not `Clone`, not `Send` (`MappedRegion` is not `Sync`): frames are made and kept on the
/// display worker. No `Hash`, `Ord` or `PartialEq`.
pub struct ConsoleFrame {
    pages: &'static MappedRegion,
    dst: ConsoleDst,
}

impl ConsoleFrame {
    /// Bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.dst.span().len()
    }

    /// Never true: a zero-length frame is refused at the mint.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// ★ The frame's memory as an opaque span — what the console publishes.
    #[must_use]
    pub fn span(&self) -> StaticSpan {
        self.dst.span()
    }

    /// A copy of `n` bytes at `off` — for a diagnostic digest of a frame whose copy COMPLETED.
    /// Bounds-checked against the frame (`MappedRegion::read_into`).
    ///
    /// # Errors
    /// Refused by name outside the frame.
    pub fn read(&self, off: usize, n: usize) -> Result<Vec<u8>, CudaError> {
        let mut out = vec![0u8; n];
        if n > 0 {
            self.pages
                .read_into(HostOffset::new(off as u64), &mut out)
                .map_err(|e| refused("ConsoleFrame::read", format!("{e}")))?;
        }
        Ok(out)
    }
}

impl core::fmt::Debug for ConsoleFrame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ConsoleFrame")
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

/// ★★★ **V10 — a layer fits its composition**: a composable rectangle (`1 ≤ rows, width ≤
/// 16384`), inside the `fw × fh` composition, and every byte the compose kernel's address function
/// can form for it inside `[src, src + extent)` — pitch: `(rows-1)·pitch + 4·width`; block-linear:
/// whole blocks of `pitch` GOBs per row over rows `[0, y0 + rows)`, with the rows' bytes inside the
/// surface's GOB columns. `kf_scanout.ptx` has no store bound of its own (it relies on this, S1-05;
/// T12 drives the address function over accepted layers).
///
/// # Errors
/// The failed bound, by name.
pub fn compose_layer_fits(l: &ComposeLayer, fw: u32, fh: u32) -> Result<(), String> {
    let fits = |a: u32, n: u32, room: u32| u64::from(a) + u64::from(n) <= u64::from(room);
    if l.rows == 0 || l.width == 0 || l.rows > MAX_SIDE || l.width > MAX_SIDE {
        return Err(format!("{l:?} is not a composable rectangle"));
    }
    if !fits(l.ox, l.width, fw) || !fits(l.oy, l.rows, fh) {
        return Err(format!("{l:?} leaves the {fw}x{fh} frame"));
    }
    let row = u64::from(l.width) * 4;
    let need = if l.block_linear {
        if l.block_height_log2 > 5
            || !l.x0_bytes.is_multiple_of(4)
            || u64::from(l.x0_bytes) + row > u64::from(l.pitch) * 64
        {
            return Err(format!("{l:?}: the rows leave the surface's GOB columns"));
        }
        (u64::from(l.y0) + u64::from(l.rows)).div_ceil(8u64 << l.block_height_log2)
            * u64::from(l.pitch)
            * (512u64 << l.block_height_log2)
    } else {
        if u64::from(l.pitch) < row {
            return Err(format!("{l:?}: a row is wider than the pitch"));
        }
        u64::from(l.rows - 1) * u64::from(l.pitch) + row
    };
    if need > l.extent {
        return Err(format!("{l:?}: the rows need {need:#x} bytes"));
    }
    Ok(())
}

/// ★ V9, pure: a `w × h` composition's bytes (`4·w·h`, in `u64`), refused outside `1..=16384`.
///
/// # Errors
/// The failed bound, by name.
pub(crate) fn composition_bytes(w: u32, h: u32) -> Result<u64, String> {
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        return Err(format!("a {w}x{h} composition"));
    }
    Ok(u64::from(w) * u64::from(h) * 4)
}

/// ★ V12 / F3, pure: a frame of `len` bytes is mintable after `minted` frames: `1 ≤ len ≤ 64 MiB`
/// and fewer than [`MAX_CONSOLE_FRAMES`] minted. Returns the size rounded up to whole host pages.
///
/// # Errors
/// The failed bound, by name.
pub(crate) fn console_frame_bytes(len: usize, minted: u32, page: u64) -> Result<u64, String> {
    let len = len as u64;
    if len == 0 || len > MAX_CONSOLE_FRAME_BYTES {
        return Err(format!(
            "a {len:#x}-byte console frame (1..={MAX_CONSOLE_FRAME_BYTES:#x})"
        ));
    }
    if minted >= MAX_CONSOLE_FRAMES {
        return Err(format!(
            "{minted} console frames already minted; frames are leaked for the process, so at \
             most {MAX_CONSOLE_FRAMES} exist (F3)"
        ));
    }
    Ok(len.div_ceil(page) * page)
}

impl DisplayGpu {
    /// Bring up a context on the GPU at PCI address `bdf` (the device's host GPU, never ordinal 0).
    ///
    /// # Errors
    /// [`CudaError`], by name.
    pub fn bring_up_on(bdf: &str) -> Result<DisplayGpu, CudaError> {
        let ctx = Ctx::create(Some(bdf))?;
        let stream = Stream::create(&ctx)?;
        let done = CompletionFd::new()?;
        let compose = Module::load(&ctx, Ptx::Scanout)
            .and_then(|m| m.kernel(COMPOSE_ENTRY))
            .map_err(|e| format!("the compose kernel did not load: {e}"));
        Ok(DisplayGpu {
            ctx,
            stream,
            done,
            compose,
            store: None,
            staging: None,
            frames_minted: 0,
            frame_bytes_minted: 0,
        })
    }

    /// Make this context current on the calling thread (the worker calls it once, at its top).
    ///
    /// # Errors
    /// [`CudaError`].
    pub fn make_current(&self) -> Result<(), CudaError> {
        self.ctx.make_current()
    }

    /// The fd that becomes readable when a queued scanout copy completed (the worker polls it).
    #[must_use]
    pub fn completion_fd(&self) -> &CompletionFd {
        &self.done
    }

    /// ★★ V2 — import the RM-exported store (`fd` on `/dev/nvidiactl`, borrowed for the call;
    /// `bytes` RM's own length). A second import is refused.
    ///
    /// # Errors
    /// [`CudaError`], naming the import step that refused.
    pub fn import_store(&mut self, fd: BorrowedFd<'_>, bytes: u64) -> Result<(), CudaError> {
        if self.store.is_some() {
            return Err(refused(
                "DisplayGpu::import_store",
                "a store is already imported".into(),
            ));
        }
        self.store = Some(DevMem::import(&self.ctx, fd, bytes)?);
        Ok(())
    }

    fn store(&self, what: &'static str) -> Result<&DevMem, CudaError> {
        self.store
            .as_ref()
            .ok_or_else(|| refused(what, "no store imported".into()))
    }

    /// ★ V1 — read `buf.len()` bytes of the store at `off` (the GPU copies them to host memory).
    ///
    /// # Errors
    /// Refused by name outside the store; the CUDA error otherwise.
    pub fn read_store(&self, off: u64, buf: &mut [u8]) -> Result<(), CudaError> {
        self.store("DisplayGpu::read_store")?.read(off, buf)
    }

    /// ★ V1 — write `bytes` into the store at `off` (a notifier or semaphore release).
    ///
    /// # Errors
    /// Refused by name outside the store; the CUDA error otherwise.
    pub fn write_store(&self, off: u64, bytes: &[u8]) -> Result<(), CudaError> {
        self.store("DisplayGpu::write_store")?.write(off, bytes)
    }

    /// ★ V1 — zero `[off, off+len)` of the store (the guest's display instance memory when it is
    /// stated: the guest never zeroes it, `disp_inst_mem.c:170-201`).
    ///
    /// # Errors
    /// Refused by name outside the store; the CUDA error otherwise.
    pub fn zero_store(&self, off: u64, len: u64) -> Result<(), CudaError> {
        self.store("DisplayGpu::zero_store")?.fill(off, len, 0)
    }

    /// ★★★ **V12 — a page-locked console frame of `len` bytes** (rounded up to whole host
    /// pages), registered with this context while still owned and leaked only once registered
    /// (`raw::console_pages`). Refused past 64 MiB or after [`MAX_CONSOLE_FRAMES`] (F3: frames are
    /// never freed, so their count and size are the leak budget).
    ///
    /// # Errors
    /// The failed bound, or the driver's refusal (nothing then stays mapped).
    pub fn console_frame(&mut self, len: usize) -> Result<ConsoleFrame, CudaError> {
        let bytes = console_frame_bytes(len, self.frames_minted, HostPageSize::query().bytes())
            .map_err(|e| refused("DisplayGpu::console_frame (V12)", e))?;
        let (pages, dst) = console_pages(&self.ctx, bytes)?;
        self.frames_minted += 1;
        self.frame_bytes_minted += bytes;
        Ok(ConsoleFrame { pages, dst })
    }

    /// Console frames minted so far, and their bytes (F3's budget line).
    #[must_use]
    pub fn console_frames_minted(&self) -> (u32, u64) {
        (self.frames_minted, self.frame_bytes_minted)
    }

    /// ★★ **V9 — start a `w × h` composition**: the device staging frame (grown when too small —
    /// the old one drains before it is freed) cleared to black on the display stream. The returned
    /// [`Composer`] holds this plane exclusively, so no second composition (which may grow the
    /// staging buffer) can begin while it lives.
    ///
    /// # Errors
    /// An empty or oversized composition, by name; the CUDA error otherwise.
    pub fn compose_begin(&mut self, w: u32, h: u32) -> Result<Composer<'_>, CudaError> {
        let n =
            composition_bytes(w, h).map_err(|e| refused("DisplayGpu::compose_begin (V9)", e))?;
        if self.staging.as_ref().is_none_or(|s| s.len() < n) {
            // the old staging drains before it is freed (a queued compose may still write it)
            self.staging = None;
            self.staging = Some(DevMem::alloc_zeroed(&self.ctx, n)?);
        }
        let c = Composer { gpu: self, w, h };
        c.staging()?.fill_async(&c.gpu.stream, 0)?;
        Ok(c)
    }

    /// ★ Bring-up self-test of the compose kernel on SYNTHETIC data (never guest memory): upload
    /// `surface` to a scratch allocation, compose layer `l` (its `src` is ignored) into a `w × h`
    /// composition, and read the staging frame back. The scratch is a `DevMem`, so it is drained
    /// before it is freed on every path.
    ///
    /// # Errors
    /// [`CudaError`]; a layer that leaves `surface` or the composition is refused (V10).
    pub fn selftest_compose(
        &mut self,
        surface: &[u8],
        l: &ComposeLayer,
        w: u32,
        h: u32,
    ) -> Result<Vec<u8>, CudaError> {
        let what = "DisplayGpu::selftest_compose";
        if l.extent > surface.len() as u64 {
            return Err(refused(what, format!("{l:?} leaves the synthetic surface")));
        }
        compose_layer_fits(l, w, h).map_err(|e| refused(what, e))?;
        let scratch = DevMem::alloc_zeroed(&self.ctx, surface.len() as u64)?;
        scratch.write(0, surface)?;
        let c = self.compose_begin(w, h)?;
        c.layer_from(scratch.range(0, l.extent)?, l)?;
        c.read_back()
    }
}

/// ★★ **One composition in progress** (V9): `(w, h)` and the staging buffer it composes into,
/// tied together for its whole life. Holds the plane exclusively.
pub struct Composer<'a> {
    gpu: &'a DisplayGpu,
    w: u32,
    h: u32,
}

impl core::fmt::Debug for Composer<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Composer")
            .field("w", &self.w)
            .field("h", &self.h)
            .finish_non_exhaustive()
    }
}

impl Composer<'_> {
    fn staging(&self) -> Result<DevRange<'_>, CudaError> {
        let n = u64::from(self.w) * u64::from(self.h) * 4;
        self.gpu
            .staging
            .as_ref()
            .ok_or_else(|| refused("Composer", "no staging buffer".into()))?
            .range(0, n)
    }

    /// ★★ **V10 — queue window `l`**: refused unless it fits this composition and its extent lies
    /// inside the store; then the compose kernel (one CTA per row, 256 threads striding it).
    ///
    /// # Errors
    /// Refused by name when the kernel did not load or a bound fails; the CUDA error otherwise.
    pub fn layer(&self, l: &ComposeLayer) -> Result<(), CudaError> {
        let what = "Composer::layer (V10)";
        compose_layer_fits(l, self.w, self.h).map_err(|e| refused(what, e))?;
        let src = self.gpu.store(what)?.range(l.src, l.extent)?;
        self.layer_from(src, l)
    }

    fn layer_from(&self, src: DevRange<'_>, l: &ComposeLayer) -> Result<(), CudaError> {
        let what = "Composer::layer (V10)";
        compose_layer_fits(l, self.w, self.h).map_err(|e| refused(what, e))?;
        if src.len() < l.extent {
            return Err(refused(
                what,
                "the source range is shorter than the extent".into(),
            ));
        }
        let k = self
            .gpu
            .compose
            .as_ref()
            .map_err(|e| refused(what, e.clone()))?;
        let dst = self.staging()?;
        // `dpitch = 4·w` cannot overflow: V9 bounds `w ≤ 16384`.
        let args = [
            Arg::Ptr(src),
            Arg::Ptr(dst),
            Arg::U32(u32::from(l.block_linear)),
            Arg::U32(l.pitch),
            Arg::U32(l.block_height_log2),
            Arg::U32(l.x0_bytes),
            Arg::U32(l.y0),
            Arg::U32(l.width),
            Arg::U32(l.ox),
            Arg::U32(l.oy),
            Arg::U32(self.w * 4),
            Arg::U32(self.w),
            Arg::U32(self.h),
            Arg::U32(l.flags),
            Arg::I32(l.a_s),
            Arg::I32(l.b_s),
            Arg::I32(l.a_d),
            Arg::I32(l.b_d),
        ];
        k.launch(&self.gpu.stream, l.rows, 256, 0, &args)
            .map(|_| ())
    }

    /// ★★ **V11 — end the composition**: the staging frame (`4·w·h` bytes, tight rows) copied into
    /// `frame`, then the host signal on [`DisplayGpu::completion_fd`]. Refused unless the frame is
    /// registered with this plane's context and holds the composition. Only the GPU moves the
    /// bytes.
    ///
    /// # Errors
    /// The failed check, or the CUDA error.
    pub fn finish(self, frame: &ConsoleFrame) -> Result<(), CudaError> {
        if frame.dst.ctx_id() != self.gpu.ctx.id() {
            return Err(refused(
                "Composer::finish (V11)",
                "the frame is registered with another CUDA context".into(),
            ));
        }
        let n = u64::from(self.w) * u64::from(self.h) * 4;
        if n > frame.len() as u64 {
            return Err(refused(
                "Composer::finish (V11)",
                format!("{n:#x} bytes do not fit a {:#x}-byte frame", frame.len()),
            ));
        }
        self.staging()?
            .copy_to_console(&self.gpu.stream, &frame.dst)?;
        self.gpu.stream.host_signal(&self.gpu.done)
    }

    /// The self-test's read-back: drain the context, then a synchronous copy of the composition.
    fn read_back(self) -> Result<Vec<u8>, CudaError> {
        self.gpu.ctx.drain()?;
        let n = u64::from(self.w) * u64::from(self.h) * 4;
        let mut out =
            vec![0u8; usize::try_from(n).map_err(|_| refused("Composer", "size".into()))?];
        self.gpu
            .staging
            .as_ref()
            .ok_or_else(|| refused("Composer", "no staging buffer".into()))?
            .read(0, &mut out)?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bl() -> ComposeLayer {
        ComposeLayer {
            src: 0,
            extent: 9 * 120 * 16 * 512,
            block_linear: true,
            pitch: 120,
            block_height_log2: 4,
            x0_bytes: 0,
            y0: 0,
            width: 1920,
            rows: 1080,
            ox: 0,
            oy: 0,
            flags: 4,
            a_s: 255,
            b_s: 0,
            a_d: 0,
            b_d: 0,
        }
    }

    /// ⊘ The compose launch passes EIGHTEEN by-value parameters (two u64, twelve u32, four s32)
    /// in this order; the PTX entry must declare exactly those — a drift is a wild pointer.
    #[test]
    fn the_compose_kernel_declares_the_parameters_the_launch_passes() {
        let ptx = std::str::from_utf8(crate::display::SCANOUT_PTX).unwrap();
        assert!(
            ptx.is_ascii(),
            "the PTX parser refuses other bytes, even in comments"
        );
        let head = ptx
            .split(&format!(".visible .entry {COMPOSE_ENTRY}("))
            .nth(1)
            .expect("the entry");
        let params: Vec<(&str, &str)> = head
            .split(')')
            .next()
            .unwrap()
            .split(',')
            .map(|p| {
                let w: Vec<&str> = p.split_whitespace().collect();
                (w[1], w[2].rsplit('_').next().unwrap())
            })
            .collect();
        let want = [
            (".u64", "src"),
            (".u64", "dst"),
            (".u32", "layout"),
            (".u32", "pitch"),
            (".u32", "bh"),
            (".u32", "x0b"),
            (".u32", "y0"),
            (".u32", "width"),
            (".u32", "ox"),
            (".u32", "oy"),
            (".u32", "dpitch"),
            (".u32", "fw"),
            (".u32", "fh"),
            (".u32", "flags"),
            (".s32", "as"),
            (".s32", "bs"),
            (".s32", "ad"),
            (".s32", "bd"),
        ];
        assert_eq!(params, want);
        assert!(
            ptx.contains(".target sm_75"),
            "Turing+ JIT target (sec. 21)"
        );
    }

    /// ★ T11 — V10: reads in the layer's extent, writes in the composition.
    #[test]
    fn a_compose_launch_is_bounded_before_it_is_queued() {
        let bl = bl();
        assert_eq!(compose_layer_fits(&bl, 1920, 1080), Ok(()));
        assert!(
            compose_layer_fits(&bl, 1919, 1080).is_err(),
            "wider than the frame"
        );
        let mut t = bl;
        t.width = 1921;
        assert!(
            compose_layer_fits(&t, 1920, 1080).is_err(),
            "a 1921x1080 layer in 1920x1080"
        );
        let mut t = bl;
        t.extent -= 1;
        assert!(
            compose_layer_fits(&t, 1920, 1080).is_err(),
            "one byte short of the blocks"
        );
        let mut t = bl;
        t.pitch = 119;
        assert!(
            compose_layer_fits(&t, 1920, 1080).is_err(),
            "rows wider than the surface"
        );
        let mut t = bl;
        (t.ox, t.width) = (1800, 250);
        assert!(
            compose_layer_fits(&t, 1920, 1080).is_err(),
            "unclipped past the right edge"
        );
        let p = ComposeLayer {
            block_linear: false,
            pitch: 7680,
            extent: 1079 * 7680 + 7680,
            ..bl
        };
        assert_eq!(compose_layer_fits(&p, 1920, 1080), Ok(()));
        let mut t = p;
        t.pitch = 7676;
        assert!(
            compose_layer_fits(&t, 1920, 1080).is_err(),
            "a row wider than the pitch"
        );
        let mut t = p;
        t.extent -= 1;
        assert!(
            compose_layer_fits(&t, 1920, 1080).is_err(),
            "pitch: one byte short"
        );
        let mut t = p;
        (t.rows, t.width) = (0, 1);
        assert!(
            compose_layer_fits(&t, 1920, 1080).is_err(),
            "an empty rectangle"
        );
    }

    /// ★ T10 — V9: the composition's geometry.
    #[test]
    fn a_composition_is_bounded_on_both_sides() {
        assert_eq!(composition_bytes(16384, 16384), Ok(16384 * 16384 * 4));
        assert!(composition_bytes(16385, 1).is_err());
        assert!(composition_bytes(1, 16385).is_err());
        assert!(composition_bytes(0, 1).is_err());
        assert!(composition_bytes(1, 0).is_err());
    }

    /// ★ T23 — V12 / F3: the leak budget.
    #[test]
    fn console_frames_are_capped_in_count_and_size() {
        let page = 4096;
        assert_eq!(
            console_frame_bytes(1, 0, page),
            Ok(4096),
            "rounded up to a page"
        );
        assert_eq!(
            console_frame_bytes(64 << 20, 15, page),
            Ok(64 << 20),
            "the exact fit"
        );
        assert!(
            console_frame_bytes((64 << 20) + 1, 0, page).is_err(),
            "64 MiB + 1"
        );
        assert!(console_frame_bytes(0, 0, page).is_err(), "empty");
        assert!(
            console_frame_bytes(4096, 16, page).is_err(),
            "the 17th frame"
        );
        let worst = u64::from(MAX_CONSOLE_FRAMES) * MAX_CONSOLE_FRAME_BYTES;
        assert_eq!(worst, 1 << 30, "the worst-case leak is 1 GiB, stated");
    }

    /// The model of `kf_scanout.ptx`'s address function, as the PTX computes it (`u32` where the
    /// PTX is `u32`): for row `row` and pixel `x`, the byte offset from `src` the kernel reads and
    /// the destination offset it writes — `None` where the kernel stops (`oy + row ≥ fh`,
    /// `ox + x ≥ fw`).
    fn pixel(l: &ComposeLayer, fw: u32, fh: u32, row: u32, x: u32) -> Option<(u64, u64)> {
        let dy = l.oy.wrapping_add(row);
        let dx = l.ox.wrapping_add(x);
        if dy >= fh || dx >= fw {
            return None;
        }
        let src = if l.block_linear {
            let xb = l.x0_bytes.wrapping_add(x << 2);
            let y = l.y0.wrapping_add(row);
            kf_disp::scanout::bl_offset(
                u64::from(xb),
                u64::from(y),
                u64::from(l.pitch),
                l.block_height_log2,
            )
        } else {
            u64::from(row) * u64::from(l.pitch) + u64::from(x << 2)
        };
        let dst = u64::from(dy) * u64::from(fw * 4) + u64::from(dx) * 4;
        Some((src, dst))
    }

    /// Every pixel the kernel reads stays below `extent` and every write below `4·fw·fh` — at the
    /// rectangle's corners, its middle, and the last GOB column and row of a block-linear surface.
    fn reads_inside(l: &ComposeLayer, fw: u32, fh: u32) -> Result<(), String> {
        let rows = [0, l.rows / 2, l.rows - 1];
        let xs = [0, l.width / 3, l.width / 2, l.width - 1];
        for &r in &rows {
            for &x in &xs {
                if let Some((s, d)) = pixel(l, fw, fh, r, x) {
                    if s + 4 > l.extent {
                        return Err(format!("{l:?} reads [{s:#x}, +4) past {:#x}", l.extent));
                    }
                    if d + 4 > u64::from(fw) * u64::from(fh) * 4 {
                        return Err(format!("{l:?} writes past the {fw}x{fh} composition"));
                    }
                }
            }
        }
        Ok(())
    }

    /// The smallest extent V10 accepts for `l` (binary search over `compose_layer_fits` itself,
    /// so the sweep drives exactly V10's boundary, not a second statement of its formula).
    fn least_extent(l: &ComposeLayer, fw: u32, fh: u32) -> Option<u64> {
        let fits = |e: u64| compose_layer_fits(&ComposeLayer { extent: e, ..*l }, fw, fh).is_ok();
        if !fits(1 << 44) {
            return None;
        }
        let (mut lo, mut hi) = (0u64, 1u64 << 44);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if fits(mid) { hi = mid } else { lo = mid + 1 }
        }
        Some(lo)
    }

    /// ★★★ **T12 — S1-05(b): the compose kernel's address function stays inside what V10
    /// accepts.** A seeded sweep of 200 000 random layers at V10's OWN least accepted extent (and
    /// some with slack), through the PTX's address arithmetic (pitch `row·pitch + 4x`;
    /// block-linear `kf_disp::scanout::bl_offset`): every byte read lies below the extent, every
    /// byte written inside the composition. ⊘ A V10 that accepted one byte too little (`need - 1`)
    /// fails here: a pitch layer's last pixel reads exactly the extent's last byte.
    #[test]
    fn the_compose_address_function_stays_inside_every_accepted_layer() {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = |m: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % m.max(1)
        };
        let mut checked = 0u64;
        for i in 0..200_000u32 {
            let fw = 1 + next(4096) as u32;
            let fh = 1 + next(2160) as u32;
            let width = 1 + next(u64::from(fw)) as u32;
            let rows = 1 + next(u64::from(fh)) as u32;
            let mut l = ComposeLayer {
                src: 0,
                extent: 0,
                block_linear: i % 2 == 0,
                pitch: 0,
                block_height_log2: next(6) as u32,
                x0_bytes: 4 * next(64) as u32,
                y0: next(64) as u32,
                width,
                rows,
                ox: next(u64::from(fw - width + 1)) as u32,
                oy: next(u64::from(fh - rows + 1)) as u32,
                flags: 0,
                a_s: 255,
                b_s: 0,
                a_d: 0,
                b_d: 0,
            };
            l.pitch = if l.block_linear {
                (u64::from(l.x0_bytes) + u64::from(width) * 4).div_ceil(64) as u32 + next(4) as u32
            } else {
                width * 4 + 4 * next(64) as u32
            };
            let Some(least) = least_extent(&l, fw, fh) else {
                continue;
            };
            l.extent = least + if i % 3 == 0 { 0 } else { next(4096) };
            reads_inside(&l, fw, fh).unwrap();
            checked += 1;
        }
        assert!(
            checked > 150_000,
            "the sweep exercised {checked} accepted layers"
        );
        // The known positive, by construction: a pitch layer one byte short of its least accepted
        // extent reads past it, so the checker is not vacuous.
        let edge = ComposeLayer {
            block_linear: false,
            pitch: 64,
            width: 16,
            rows: 8,
            ..bl()
        };
        let least = least_extent(&edge, 16, 8).expect("a fitting layer");
        assert_eq!(least, 7 * 64 + 64);
        let short = ComposeLayer {
            extent: least - 1,
            ..edge
        };
        assert!(
            reads_inside(&short, 16, 8).is_err(),
            "the checker sees the byte past the extent"
        );
    }
}
