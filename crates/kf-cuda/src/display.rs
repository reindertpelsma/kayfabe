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
    CUdeviceptr, CompletionFd, CtxHandle, Cuda, CudaError, PinnedBuf, StreamHandle,
};

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
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frame").field("len", &self.len()).finish()
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
        Ok(DisplayGpu {
            cu,
            ctx,
            device,
            store: None,
            stream,
            done,
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
