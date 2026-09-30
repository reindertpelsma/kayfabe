//! ★ The display plane's own GPU context — how the emulated display engine reaches the guest's
//! framebuffer (the store) WITHOUT the CPU (`THE_CONSTRAINTS.md` §38: *"the CPU never reads guest
//! vidmem"*; `docs/design/V3_DISPLAY.md` §4.3–§4.6).
//!
//! The display engine is an emulated device, and the store is its memory: the context-DMA table in
//! the guest's display instance memory (FB) is read here, a video-memory notifier or semaphore is
//! written here, and (M2) a flipped surface is copied out of the store here by the GPU. Every access
//! is bounds-checked against the imported store before a byte moves; the device pointer never leaves
//! this struct (§13).
//!
//! ⊘ A SEPARATE context from the walker's (`walk.rs`): the walker is owned by the VA thread and its
//! synchronous accesses order behind walks; the display worker must never wait on a walk, and a
//! walk must never wait on a scanout copy.

use crate::driver_unsafe::{
    CUdeviceptr, CompletionFd, CtxHandle, Cuda, CudaError, Func, PinnedBuf, StreamHandle,
};

/// ★ The display plane's kernels, hand-written PTX (`cuda/display/kf_scanout.ptx`), JIT-compiled at
/// the plane's bring-up; the block-linear address function is `kf_disp::scanout::bl_offset`.
pub static SCANOUT_PTX: &[u8] = include_bytes!("../../../cuda/display/kf_scanout.ptx");

/// The compose kernel's entry point: one window into the head's staging frame.
pub const COMPOSE_ENTRY: &str = "kf_compose";

/// ★ The display worker's GPU context with the store imported.
pub struct DisplayGpu {
    cu: Cuda,
    ctx: CtxHandle,
    device: i32,
    store: Option<(CUdeviceptr, u64)>,
    /// The scanout copies' stream (blocking, like every stream this binding creates).
    stream: StreamHandle,
    /// Readable once the last queued scanout copy completed (`cuLaunchHostFunc` after it).
    done: CompletionFd,
    /// The compose kernel, or why it did not load (every scanout is refused by it).
    compose: Result<Func, String>,
    /// The device staging frame the windows compose into: address and bytes.
    staging: Option<(CUdeviceptr, usize)>,
}

/// ★ One page-locked host frame buffer the display console reads (M2). Its address crosses to the
/// QEMU console as an integer; this crate writes it only through the GPU copy engine.
pub struct Frame {
    buf: PinnedBuf,
}

impl Frame {
    /// Host address of the frame's first byte.
    #[must_use]
    pub fn addr(&self) -> usize {
        self.buf.addr()
    }

    /// Bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Never empty (a zero-length frame is refused at allocation).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buf.len() == 0
    }

    /// A copy of `n` bytes at `off` — for a diagnostic digest of a frame whose copy COMPLETED
    /// (the console reads the same bytes to show them). ⊘ Never while a copy targets the frame.
    ///
    /// # Panics
    /// If the range leaves the frame.
    #[must_use]
    pub fn read(&self, off: usize, n: usize) -> Vec<u8> {
        self.buf.read(off, n)
    }
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frame").field("len", &self.len()).finish()
    }
}

/// ★ One window's compose-kernel program (the mirror of `kf_disp::scanout::LayerPlan`): every read
/// is bounded by `extent` inside the store and every write by the frame, before it is queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComposeLayer {
    /// Store offset of the first byte the kernel addresses.
    pub src: u64,
    /// Bytes from `src` the kernel may read.
    pub extent: u64,
    /// Block-linear (else pitch).
    pub block_linear: bool,
    /// Pitch in bytes, or GOBs per row.
    pub pitch: u32,
    /// log2 GOBs per block.
    pub block_height_log2: u32,
    /// The rectangle's first byte column (block-linear).
    pub x0_bytes: u32,
    /// Its first row (block-linear).
    pub y0: u32,
    /// Pixels per row.
    pub width: u32,
    /// Rows.
    pub rows: u32,
    /// Output column in the frame.
    pub ox: u32,
    /// Output row.
    pub oy: u32,
    /// `bit0` alpha, `bit1` swap red/blue, `bit2` opaque.
    pub flags: u32,
    /// Blend coefficients (see `kf_compose`).
    pub a_s: i32,
    /// See `a_s`.
    pub b_s: i32,
    /// See `a_s`.
    pub a_d: i32,
    /// See `a_s`.
    pub b_d: i32,
}

impl ComposeLayer {
    /// ⊘ The kernel's reads stay inside `extent` and its writes inside a `fw` x `fh` frame.
    fn check(&self, fw: u32, fh: u32) -> Result<(), String> {
        let l = *self;
        let fits = |a: u32, n: u32, room: u32| u64::from(a) + u64::from(n) <= u64::from(room);
        if l.rows == 0 || l.width == 0 || l.rows > 16384 || l.width > 16384 {
            return Err(format!("{l:?} is not a composable rectangle"));
        }
        if !fits(l.ox, l.width, fw) || !fits(l.oy, l.rows, fh) {
            return Err(format!("{l:?} leaves the {fw}x{fh} frame"));
        }
        let row = u64::from(l.width) * 4;
        let need = if l.block_linear {
            if l.block_height_log2 > 5
                || l.x0_bytes % 4 != 0
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
}

impl std::fmt::Debug for DisplayGpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DisplayGpu")
            .field("device", &self.device)
            .field("store_bytes", &self.store.map(|s| s.1))
            .finish()
    }
}

fn refused(what: &'static str, name: String) -> CudaError {
    CudaError::Refused {
        what,
        code: 0,
        name,
    }
}

impl DisplayGpu {
    /// Bring up a context on the GPU at PCI address `bdf` (the device's host GPU, never ordinal 0).
    ///
    /// # Errors
    /// [`CudaError`], by name.
    pub fn bring_up_on(bdf: &str) -> Result<DisplayGpu, CudaError> {
        let cu = Cuda::open()?;
        cu.init()?;
        let device = cu.device_by_pci_bus_id(bdf)?;
        let ctx = cu.ctx_create(device)?;
        let stream = cu.stream_create()?;
        let done = CompletionFd::new()?;
        let mut ptx = SCANOUT_PTX.to_vec();
        ptx.push(0);
        let compose = cu
            .module_load(&ptx)
            .and_then(|m| cu.module_function(m, COMPOSE_ENTRY))
            .map_err(|e| format!("the compose kernel did not load: {e}"));
        Ok(DisplayGpu {
            cu,
            ctx,
            device,
            store: None,
            stream,
            done,
            compose,
            staging: None,
        })
    }

    /// Allocate a page-locked frame of `len` bytes in this context.
    ///
    /// # Errors
    /// [`CudaError`]; a zero length is refused.
    pub fn frame(&self, len: usize) -> Result<Frame, CudaError> {
        if len == 0 {
            return Err(refused("DisplayGpu::frame", "a zero-length frame".into()));
        }
        self.make_current()?;
        Ok(Frame {
            buf: self.cu.pinned_alloc(len, "cuMemAllocHost(display frame)")?,
        })
    }

    /// The fd that becomes readable when a queued scanout copy completed (the worker polls it).
    #[must_use]
    pub fn completion_fd(&self) -> &CompletionFd {
        &self.done
    }

    /// ★ Give a frame back (`cuMemFreeHost`). ⊘ Only once no queued copy targets it and the console
    /// no longer reads it — the caller's frame state machine decides that, never this binding.
    ///
    /// # Errors
    /// [`CudaError`].
    pub fn release_frame(&self, f: Frame) -> Result<(), CudaError> {
        self.make_current()?;
        self.cu.pinned_free(f.buf, "cuMemFreeHost(display frame)")
    }

    /// ★ Start a `w` x `h` composition: the device staging frame (grown when too small) cleared to
    /// black on the display stream — what no window covers is black, as on a real head.
    ///
    /// # Errors
    /// [`CudaError`]; an empty or oversized frame is refused.
    pub fn compose_begin(&mut self, w: u32, h: u32) -> Result<(), CudaError> {
        let what = "DisplayGpu::compose_begin";
        let n = usize::try_from(u64::from(w) * u64::from(h) * 4)
            .map_err(|_| refused(what, format!("{w}x{h}")))?;
        if n == 0 || w > 16384 || h > 16384 {
            return Err(refused(what, format!("a {w}x{h} frame")));
        }
        if self.staging.is_none_or(|(_, len)| len < n) {
            if let Some((p, _)) = self.staging.take() {
                // the stream drains first: a queued compose may still write the old staging
                self.cu.ctx_synchronize()?;
                self.cu.mem_free(p);
            }
            self.staging = Some((
                self.cu.mem_alloc_zeroed(n, "cuMemAlloc(display staging)")?,
                n,
            ));
        }
        let (p, _) = self.staging.unwrap_or((0, 0));
        self.cu
            .memset_d8_async(self.stream, p, 0, n, "cuMemsetD8Async(display staging)")
    }

    /// ★ Queue window `l` of the composition begun by [`Self::compose_begin`] (`w` x `h`).
    ///
    /// # Errors
    /// Refused by name when the kernel did not load, a read would leave the store or the layer's
    /// extent, or a write the frame; the CUDA error otherwise.
    pub fn compose_layer(&self, l: &ComposeLayer, w: u32, h: u32) -> Result<(), CudaError> {
        let what = "DisplayGpu::compose_layer";
        l.check(w, h).map_err(|e| refused(what, e))?;
        let n = usize::try_from(l.extent).map_err(|_| refused(what, format!("{l:?}")))?;
        let src = self.at(l.src, n, what)?;
        self.launch_compose(src, l, w, h, what)
    }

    /// ★ End the composition: the staging frame is copied into `dst` (tight, `w * 4` bytes a row),
    /// then the host signal on [`Self::completion_fd`]. Only the GPU moves the bytes.
    ///
    /// # Errors
    /// Refused by name when the frame is smaller than the composition; the CUDA error otherwise.
    pub fn compose_finish(&self, w: u32, h: u32, dst: &Frame) -> Result<(), CudaError> {
        let what = "DisplayGpu::compose_finish";
        let n = usize::try_from(u64::from(w) * u64::from(h) * 4)
            .map_err(|_| refused(what, format!("{w}x{h}")))?;
        let Some((p, len)) = self.staging else {
            return Err(refused(what, "no composition was begun".into()));
        };
        if n > len || n > dst.len() {
            return Err(refused(what, format!("{n:#x} bytes do not fit")));
        }
        self.cu.memcpy_d2h_async(
            self.stream,
            &dst.buf,
            0,
            p,
            n,
            "cuMemcpyDtoHAsync(display frame)",
        )?;
        self.cu.launch_host_signal(self.stream, &self.done)
    }

    /// The compose kernel launch (one CTA per row, 256 threads striding the row).
    fn launch_compose(
        &self,
        src: CUdeviceptr,
        l: &ComposeLayer,
        w: u32,
        h: u32,
        what: &'static str,
    ) -> Result<(), CudaError> {
        let f = *self
            .compose
            .as_ref()
            .map_err(|e| refused(what, e.clone()))?;
        let Some((dst, _)) = self.staging else {
            return Err(refused(what, "no composition was begun".into()));
        };
        let u = |x: u32| x.to_le_bytes().to_vec();
        let i = |x: i32| x.to_le_bytes().to_vec();
        let mut params = vec![
            src.to_le_bytes().to_vec(),
            dst.to_le_bytes().to_vec(),
            u(u32::from(l.block_linear)),
            u(l.pitch),
            u(l.block_height_log2),
            u(l.x0_bytes),
            u(l.y0),
            u(l.width),
            u(l.ox),
            u(l.oy),
            u(w * 4),
            u(w),
            u(h),
            u(l.flags),
            i(l.a_s),
            i(l.b_s),
            i(l.a_d),
            i(l.b_d),
        ];
        self.cu
            .launch_args(self.stream, f, l.rows, 256, 0, &mut params, what)
    }

    /// ★ Bring-up self-test of the compose kernel on SYNTHETIC data (never guest memory): upload
    /// `surface` to a scratch allocation, compose layer `l` (its `src` is ignored) into a `w` x `h`
    /// frame, wait, and return the frame's bytes — the caller compares them with the reference.
    ///
    /// # Errors
    /// [`CudaError`]; a layer that leaves `surface` or the frame is refused.
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
        l.check(w, h).map_err(|e| refused(what, e))?;
        self.make_current()?;
        let len = usize::try_from(u64::from(w) * u64::from(h) * 4)
            .map_err(|_| refused(what, format!("{w}x{h}")))?;
        let scratch = self
            .cu
            .mem_alloc_zeroed(surface.len(), "cuMemAlloc(compose selftest)")?;
        let out = (|| {
            self.cu
                .memcpy_h2d(scratch, surface, "cuMemcpyHtoD(compose selftest)")?;
            let frame = self.frame(len)?;
            let run = self
                .compose_begin(w, h)
                .and_then(|()| self.launch_compose(scratch, l, w, h, what))
                .and_then(|()| {
                    let (p, _) = self.staging.unwrap_or((0, 0));
                    self.cu.memcpy_d2h_async(
                        self.stream,
                        &frame.buf,
                        0,
                        p,
                        len,
                        "cuMemcpyDtoHAsync(compose selftest)",
                    )
                })
                .and_then(|()| self.cu.ctx_synchronize());
            let bytes = run.map(|()| frame.read(0, len));
            let freed = self.release_frame(frame);
            let bytes = bytes?;
            freed?;
            Ok(bytes)
        })();
        self.cu.mem_free(scratch);
        out
    }

    /// Make this context current on the calling thread (the worker calls it once, at its top).
    ///
    /// # Errors
    /// [`CudaError`].
    pub fn make_current(&self) -> Result<(), CudaError> {
        self.cu.ctx_set_current(self.ctx)
    }

    /// Import the RM-exported store (`fd` on `/dev/nvidiactl`, `bytes` long) into this context.
    ///
    /// # Errors
    /// [`CudaError`], naming the import step that refused; a second import is refused.
    pub fn import_store(&mut self, fd: i32, bytes: u64) -> Result<(), CudaError> {
        if self.store.is_some() {
            return Err(refused(
                "DisplayGpu::import_store",
                "a store is already imported".into(),
            ));
        }
        self.make_current()?;
        let n = usize::try_from(bytes).map_err(|_| {
            refused(
                "DisplayGpu::import_store",
                format!("{bytes:#x} does not fit usize"),
            )
        })?;
        let p = self
            .cu
            .import_and_map(self.device, fd, n)
            .map_err(|name| CudaError::Refused {
                what: "cuMemImportFromShareableHandle + cuMemMap (display)",
                code: 0,
                name,
            })?;
        self.store = Some((p, bytes));
        Ok(())
    }

    /// The store address of `[off, off+len)`, refused by name unless wholly inside the store.
    fn at(&self, off: u64, len: usize, what: &'static str) -> Result<CUdeviceptr, CudaError> {
        let Some((base, bytes)) = self.store else {
            return Err(refused(what, "no store imported".into()));
        };
        if off.checked_add(len as u64).is_none_or(|e| e > bytes) {
            return Err(refused(
                what,
                format!("[{off:#x}, +{len:#x}) leaves the {bytes:#x}-byte store"),
            ));
        }
        Ok(base + off)
    }

    /// ★ Read `buf.len()` bytes of the store at `off` — the GPU copies them to host memory (the
    /// display instance memory's context-DMA table, a video-memory semaphore).
    ///
    /// # Errors
    /// Refused by name outside the store; the CUDA error otherwise.
    pub fn read_store(&self, off: u64, buf: &mut [u8]) -> Result<(), CudaError> {
        let src = self.at(off, buf.len(), "DisplayGpu::read_store")?;
        self.cu
            .memcpy_d2h(buf, src, "cuMemcpyDtoH(display read_store)")
    }

    /// ★ Write `bytes` into the store at `off` (a video-memory notifier or semaphore release).
    ///
    /// # Errors
    /// Refused by name outside the store; the CUDA error otherwise.
    pub fn write_store(&self, off: u64, bytes: &[u8]) -> Result<(), CudaError> {
        let dst = self.at(off, bytes.len(), "DisplayGpu::write_store")?;
        self.cu
            .memcpy_h2d(dst, bytes, "cuMemcpyHtoD(display write_store)")
    }

    /// ★ Zero `[off, off+len)` of the store (the guest's display instance memory when it is stated:
    /// the guest never zeroes it, `disp_inst_mem.c:170-201`).
    ///
    /// # Errors
    /// Refused by name outside the store; the CUDA error otherwise.
    pub fn zero_store(&self, off: u64, len: u64) -> Result<(), CudaError> {
        let n = usize::try_from(len)
            .map_err(|_| refused("DisplayGpu::zero_store", format!("{len:#x}")))?;
        let dst = self.at(off, n, "DisplayGpu::zero_store")?;
        self.cu
            .memset_d8(dst, 0, n, "cuMemsetD8(display zero_store)")
    }
}

impl Drop for DisplayGpu {
    fn drop(&mut self) {
        // the stream drains (every queued host signal with it) before the context goes
        self.cu.stream_destroy(self.stream);
        if let Some((p, _)) = self.staging.take() {
            self.cu.mem_free(p);
        }
        self.cu.ctx_destroy(self.ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⊘ The compose launch passes EIGHTEEN by-value parameters (two u64, twelve u32, four s32) in
    /// this order; the PTX entry must declare exactly those — a drift is a wild pointer on the GPU.
    #[test]
    fn the_compose_kernel_declares_the_parameters_the_launch_passes() {
        let ptx = std::str::from_utf8(SCANOUT_PTX).unwrap();
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

    /// The host-side bounds of a compose launch: reads in the layer's extent, writes in the frame.
    #[test]
    fn a_compose_launch_is_bounded_before_it_is_queued() {
        let bl = ComposeLayer {
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
        };
        assert_eq!(bl.check(1920, 1080), Ok(()));
        assert!(bl.check(1919, 1080).is_err(), "wider than the frame");
        let mut t = bl;
        t.extent -= 1;
        assert!(t.check(1920, 1080).is_err(), "one byte short of the blocks");
        let mut t = bl;
        t.pitch = 119;
        assert!(t.check(1920, 1080).is_err(), "rows wider than the surface");
        let mut t = bl;
        (t.ox, t.width) = (1800, 250);
        assert!(
            t.check(1920, 1080).is_err(),
            "unclipped past the right edge"
        );
        let p = ComposeLayer {
            block_linear: false,
            pitch: 7680,
            extent: 1079 * 7680 + 7680,
            ..bl
        };
        assert_eq!(p.check(1920, 1080), Ok(()));
        let mut t = p;
        t.pitch = 7676;
        assert!(t.check(1920, 1080).is_err(), "a row wider than the pitch");
    }
}
