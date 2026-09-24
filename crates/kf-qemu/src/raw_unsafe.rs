//! ★ The ONE raw-memory type: a region QEMU owns (BAR0 shadow RAM, guest RAM) that Rust reads and
//! writes. Everything else in this crate is safe code over it.

/// `[ptr, ptr+len)` of memory QEMU allocated and keeps mapped for the device's lifetime.
#[derive(Debug, Clone, Copy)]
pub struct RawRegion {
    ptr: *mut u8,
    len: usize,
}

// SAFETY: the region is process memory QEMU keeps mapped from registration until the device (or
// the RAM block) is torn down, which unregisters it here first. Concurrent access is the device's
// ordinary model (a guest vCPU and our threads touch the same page, as hardware DMA does); every
// access below is a bounds-checked volatile or byte copy, never a reference.
unsafe impl Send for RawRegion {}
// SAFETY: as above — no `&`/`&mut` into the memory is ever created, only volatile/copy access.
unsafe impl Sync for RawRegion {}

impl RawRegion {
    /// Adopt `[ptr, ptr+len)`.
    ///
    /// # Safety
    /// `ptr` must be valid for reads and writes of `len` bytes until this region is dropped from
    /// every structure holding it (the device unregisters it before QEMU frees the memory).
    #[must_use]
    pub unsafe fn adopt(ptr: *mut u8, len: usize) -> RawRegion {
        RawRegion { ptr, len }
    }

    /// Length in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the region is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn span(&self, off: usize, n: usize) -> Option<*mut u8> {
        let end = off.checked_add(n)?;
        // SAFETY: `off + n <= len` is checked here, so the pointer stays inside the adopted region.
        (end <= self.len).then(|| unsafe { self.ptr.add(off) })
    }

    /// Store a 32-bit value (volatile — a guest vCPU may read it concurrently).
    pub fn store_u32(&self, off: usize, v: u32) -> bool {
        let Some(p) = self.span(off, 4) else { return false };
        // SAFETY: `span` bounds-checked 4 bytes; unaligned-safe write of a plain integer.
        unsafe { core::ptr::write_unaligned(p.cast::<u32>(), v) };
        true
    }

    /// Load a 32-bit value.
    #[must_use]
    pub fn load_u32(&self, off: usize) -> Option<u32> {
        let p = self.span(off, 4)?;
        // SAFETY: `span` bounds-checked 4 bytes; unaligned-safe read of a plain integer.
        Some(unsafe { core::ptr::read_unaligned(p.cast::<u32>()) })
    }

    /// Store `width` (1, 2, 4 or 8) bytes of `v`, little-endian.
    pub fn store(&self, off: usize, v: u64, width: u8) -> bool {
        let n = usize::from(width);
        if !matches!(n, 1 | 2 | 4 | 8) {
            return false;
        }
        self.write_from(off, &v.to_le_bytes()[..n])
    }

    /// Copy `src` into the region at `off`.
    pub fn write_from(&self, off: usize, src: &[u8]) -> bool {
        let Some(p) = self.span(off, src.len()) else { return false };
        // SAFETY: `span` bounds-checked `src.len()` bytes; `src` is a Rust slice, which cannot alias
        // guest memory we never hand out references to.
        unsafe { core::ptr::copy_nonoverlapping(src.as_ptr(), p, src.len()) };
        true
    }

    /// Copy from the region at `off` into `dst`.
    pub fn read_into(&self, off: usize, dst: &mut [u8]) -> bool {
        let Some(p) = self.span(off, dst.len()) else { return false };
        // SAFETY: `span` bounds-checked `dst.len()` bytes; `dst` is ours alone.
        unsafe { core::ptr::copy_nonoverlapping(p, dst.as_mut_ptr(), dst.len()) };
        true
    }
}

/// ★ P4: borrow a descriptor QEMU owns for the life of the process — a memory backend's fd
/// (`memory-backend-memfd`), which the device maps guest RAM from into its CPU windows.
///
/// ⊘ Never closed here; the borrow is only as long as one `mmap` call needs it.
#[must_use]
pub fn borrow_process_fd(fd: i32) -> std::os::fd::BorrowedFd<'static> {
    // SAFETY: `fd` is a memory backend's descriptor that QEMU registered with this device
    // (`kf3_ram_add`) and keeps open until the RAM block is torn down, which unregisters it here
    // first (`kf3_ram_del`); callers only reach this after looking the fd up in the live
    // `RamMap`, and use the borrow for one `mmap`. `BorrowedFd` never closes it.
    unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) }
}
