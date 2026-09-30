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

/// ★ The block-linear scanout kernel, hand-written PTX (`cuda/display/kf_scanout.ptx`), JIT-compiled
/// at the plane's bring-up; its address function is `kf_disp::scanout::bl_offset`.
pub static SCANOUT_PTX: &[u8] = include_bytes!("../../../cuda/display/kf_scanout.ptx");

/// The kernel's entry point.
pub const SCANOUT_ENTRY: &str = "kf_bl_to_pitch";

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
    /// The block-linear kernel, or why it did not load (block-linear scanouts are refused by it).
    bl: Result<Func, String>,
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

/// ★ A block-linear surface's rectangle to un-swizzle into a frame (bounds are checked before a
/// byte moves): the kernel reads `[src, src + extent)` of whole blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlRect {
    /// Store offset of the surface's first block.
    pub src: u64,
    /// Bytes of whole blocks the rectangle's rows lie in.
    pub extent: u64,
    /// The surface's width in GOBs.
    pub gobs_per_row: u32,
    /// log2 GOBs per block (0..=5).
    pub block_height_log2: u32,
    /// The rectangle's first byte column (a multiple of 4).
    pub x0_bytes: u32,
    /// Its first row.
    pub y0: u32,
    /// 4-byte words per row.
    pub words: u32,
    /// Rows.
    pub rows: u32,
    /// The frame's pitch in bytes.
    pub dst_pitch: u64,
}

impl BlRect {
    /// ⊘ The kernel's reads stay inside `extent` and its writes inside `dst_len` — or the refusal.
    fn check(&self, dst_len: usize) -> Result<(), String> {
        let r = *self;
        if r.rows == 0 || r.words == 0 || r.rows > 16384 || r.block_height_log2 > 5 {
            return Err(format!("{r:?} is not a copyable rectangle"));
        }
        let row = u64::from(r.words) * 4;
        if r.x0_bytes % 4 != 0 || u64::from(r.x0_bytes) + row > u64::from(r.gobs_per_row) * 64 {
            return Err(format!("{r:?}: the rows leave the surface's GOB columns"));
        }
        let rows_per_block = 8u64 << r.block_height_log2;
        let need = (u64::from(r.y0) + u64::from(r.rows)).div_ceil(rows_per_block)
            * u64::from(r.gobs_per_row)
            * (512u64 << r.block_height_log2);
        if need > r.extent {
            return Err(format!("{r:?}: the rows need {need:#x} bytes of blocks"));
        }
        let last = u64::from(r.rows - 1)
            .checked_mul(r.dst_pitch)
            .and_then(|x| x.checked_add(row));
        if r.dst_pitch < row || last.is_none_or(|l| l > dst_len as u64) {
            return Err(format!("{r:?}: the rows leave the {dst_len:#x}-byte frame"));
        }
        Ok(())
    }
}

/// ★ A pitch-linear rectangle to copy out of the store (bounds are checked before a byte moves).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PitchRect {
    /// Store offset of the first pixel of the first row.
    pub src: u64,
    /// The surface's pitch in bytes.
    pub src_pitch: u64,
    /// Bytes per row to copy.
    pub row_bytes: u64,
    /// Rows.
    pub rows: u32,
    /// The frame's pitch in bytes (at least `row_bytes`).
    pub dst_pitch: u64,
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
        let bl = cu
            .module_load(&ptx)
            .and_then(|m| cu.module_function(m, SCANOUT_ENTRY))
            .map_err(|e| format!("the block-linear scanout kernel did not load: {e}"));
        Ok(DisplayGpu {
            cu,
            ctx,
            device,
            store: None,
            stream,
            done,
            bl,
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

    /// ★ Queue the copy of pitch-linear rectangle `r` of the store into `dst` (one 2D copy on the
    /// display stream), then a host signal on [`Self::completion_fd`]: the GPU moves every byte;
    /// nothing is read by the CPU here.
    ///
    /// # Errors
    /// Refused by name when any byte of the source leaves the store or of the destination leaves
    /// the frame; the CUDA error otherwise.
    pub fn scanout_pitch(&self, r: PitchRect, dst: &Frame) -> Result<(), CudaError> {
        let what = "DisplayGpu::scanout_pitch";
        if r.rows == 0
            || r.row_bytes == 0
            || r.row_bytes > r.src_pitch
            || r.row_bytes > r.dst_pitch
            || r.rows > 16384
        {
            return Err(refused(what, format!("{r:?} is not a copyable rectangle")));
        }
        let last = u64::from(r.rows - 1);
        let span = |pitch: u64| {
            last.checked_mul(pitch)
                .and_then(|x| x.checked_add(r.row_bytes))
                .ok_or_else(|| refused(what, format!("{r:?} overflows")))
        };
        let (src_len, dst_len) = (span(r.src_pitch)?, span(r.dst_pitch)?);
        if dst_len > dst.len() as u64 {
            return Err(refused(
                what,
                format!(
                    "{dst_len:#x} bytes do not fit the {:#x}-byte frame",
                    dst.len()
                ),
            ));
        }
        let usz = |x: u64| usize::try_from(x).map_err(|_| refused(what, format!("{x:#x}")));
        let src = self.at(r.src, usz(src_len)?, what)?;
        self.cu.memcpy2d_d2h_async(
            self.stream,
            &dst.buf,
            0,
            usz(r.dst_pitch)?,
            src,
            usz(r.src_pitch)?,
            usz(r.row_bytes)?,
            r.rows as usize,
            "cuMemcpy2DAsync(scanout)",
        )?;
        self.cu.launch_host_signal(self.stream, &self.done)
    }

    /// ★ Queue the un-swizzle of block-linear rectangle `r` of the store into `dst` (the kernel on the
    /// display stream, writing the page-locked frame through its device mapping), then the host
    /// signal on [`Self::completion_fd`]. The CPU reads nothing.
    ///
    /// # Errors
    /// Refused by name when the kernel did not load, or any byte it would read leaves the store or
    /// the rectangle's blocks, or any byte it would write leaves the frame; the CUDA error otherwise.
    pub fn scanout_block_linear(&self, r: BlRect, dst: &Frame) -> Result<(), CudaError> {
        let what = "DisplayGpu::scanout_block_linear";
        r.check(dst.len()).map_err(|e| refused(what, e))?;
        let n = usize::try_from(r.extent).map_err(|_| refused(what, format!("{r:?}")))?;
        let src = self.at(r.src, n, what)?;
        self.launch_bl(src, r, dst, what)?;
        self.cu.launch_host_signal(self.stream, &self.done)
    }

    /// The kernel launch itself (one CTA per row, 256 threads striding the row's words).
    fn launch_bl(
        &self,
        src: CUdeviceptr,
        r: BlRect,
        dst: &Frame,
        what: &'static str,
    ) -> Result<(), CudaError> {
        let f = *self.bl.as_ref().map_err(|e| refused(what, e.clone()))?;
        let dptr = self.cu.pinned_device_ptr(&dst.buf, 0)?;
        let dpitch =
            u32::try_from(r.dst_pitch).map_err(|_| refused(what, format!("{r:?} pitch")))?;
        let mut params = vec![
            src.to_le_bytes().to_vec(),
            dptr.to_le_bytes().to_vec(),
            r.x0_bytes.to_le_bytes().to_vec(),
            r.y0.to_le_bytes().to_vec(),
            r.words.to_le_bytes().to_vec(),
            r.gobs_per_row.to_le_bytes().to_vec(),
            r.block_height_log2.to_le_bytes().to_vec(),
            dpitch.to_le_bytes().to_vec(),
        ];
        self.cu
            .launch_args(self.stream, f, r.rows, 256, 0, &mut params, what)
    }

    /// ★ Bring-up self-test of the block-linear kernel on SYNTHETIC data (never guest memory): upload
    /// `surface` to a scratch allocation, run the kernel over rectangle `r` (its `src` is ignored),
    /// wait, and return the frame's bytes — the caller compares them with the reference.
    ///
    /// # Errors
    /// [`CudaError`]; a rectangle that leaves `surface` is refused.
    pub fn selftest_block_linear(&self, surface: &[u8], r: BlRect) -> Result<Vec<u8>, CudaError> {
        let what = "DisplayGpu::selftest_block_linear";
        let len = usize::try_from(u64::from(r.rows) * r.dst_pitch)
            .map_err(|_| refused(what, format!("{r:?}")))?;
        if r.extent > surface.len() as u64 {
            return Err(refused(what, format!("{r:?} leaves the synthetic surface")));
        }
        r.check(len).map_err(|e| refused(what, e))?;
        self.make_current()?;
        let scratch = self
            .cu
            .mem_alloc_zeroed(surface.len(), "cuMemAlloc(bl selftest)")?;
        let out = (|| {
            self.cu
                .memcpy_h2d(scratch, surface, "cuMemcpyHtoD(bl selftest)")?;
            let frame = self.frame(len)?;
            let run = self
                .launch_bl(scratch, r, &frame, what)
                .and_then(|()| self.cu.ctx_synchronize());
            let bytes = run.map(|()| frame.read(0, len));
            // the kernel is done (or failed): the frame can go
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
        self.cu.ctx_destroy(self.ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⊘ The launch passes EIGHT by-value parameters (two u64 then six u32) in this order; the PTX
    /// entry must declare exactly those — a drift is a wild pointer on the GPU, not a type error.
    #[test]
    fn the_scanout_kernel_declares_the_parameters_the_launch_passes() {
        let ptx = std::str::from_utf8(SCANOUT_PTX).unwrap();
        let head = ptx
            .split(&format!(".visible .entry {SCANOUT_ENTRY}("))
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
        assert_eq!(
            params,
            [
                (".u64", "src"),
                (".u64", "dst"),
                (".u32", "x0b"),
                (".u32", "y0"),
                (".u32", "words"),
                (".u32", "gpr"),
                (".u32", "bh"),
                (".u32", "dpitch"),
            ]
        );
        assert!(ptx.contains(".target sm_75"), "Turing+ JIT target (§21)");
    }

    /// The host-side bounds of a block-linear launch: the reads stay in whole blocks, the writes in
    /// the frame.
    #[test]
    fn a_block_linear_launch_is_bounded_before_it_is_queued() {
        let r = BlRect {
            src: 0,
            extent: 9 * 120 * 16 * 512,
            gobs_per_row: 120,
            block_height_log2: 4,
            x0_bytes: 0,
            y0: 0,
            words: 1920,
            rows: 1080,
            dst_pitch: 7680,
        };
        assert_eq!(r.check(7680 * 1080), Ok(()));
        assert!(
            r.check(7680 * 1080 - 1).is_err(),
            "one byte short of the frame"
        );
        let mut t = r;
        t.extent -= 1;
        assert!(
            t.check(7680 * 1080).is_err(),
            "one byte short of the blocks"
        );
        let mut t = r;
        t.gobs_per_row = 119;
        assert!(t.check(7680 * 1080).is_err(), "rows wider than the surface");
        let mut t = r;
        t.x0_bytes = 2;
        assert!(t.check(7680 * 1080).is_err(), "not word-aligned");
        let mut t = r;
        t.block_height_log2 = 6;
        assert!(t.check(7680 * 1080).is_err());
    }
}
