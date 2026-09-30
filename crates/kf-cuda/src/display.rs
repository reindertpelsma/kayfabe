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

use crate::driver_unsafe::{CUdeviceptr, CtxHandle, Cuda, CudaError};

/// ★ The display worker's GPU context with the store imported.
pub struct DisplayGpu {
    cu: Cuda,
    ctx: CtxHandle,
    device: i32,
    store: Option<(CUdeviceptr, u64)>,
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
        Ok(DisplayGpu {
            cu,
            ctx,
            device,
            store: None,
        })
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
        self.cu.ctx_destroy(self.ctx);
    }
}
