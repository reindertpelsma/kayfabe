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
        let Some(p) = self.span(off, 4) else {
            return false;
        };
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
        let Some(p) = self.span(off, src.len()) else {
            return false;
        };
        // SAFETY: `span` bounds-checked `src.len()` bytes; `src` is a Rust slice, which cannot alias
        // guest memory we never hand out references to.
        unsafe { core::ptr::copy_nonoverlapping(src.as_ptr(), p, src.len()) };
        true
    }

    /// Copy from the region at `off` into `dst`.
    pub fn read_into(&self, off: usize, dst: &mut [u8]) -> bool {
        let Some(p) = self.span(off, dst.len()) else {
            return false;
        };
        // SAFETY: `span` bounds-checked `dst.len()` bytes; `dst` is ours alone.
        unsafe { core::ptr::copy_nonoverlapping(p, dst.as_mut_ptr(), dst.len()) };
        true
    }
}

/// ★ P4: a memory backend's descriptor (`memory-backend-memfd`) that QEMU registered with this
/// device — a TYPED token, so safe code can only hold fds that crossed the FFI as one
/// (`THE_CONSTRAINTS.md` §13). Minted only by the `unsafe` [`BackendFd::adopt`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendFd(i32);

impl BackendFd {
    /// Adopt `fd`; `None` for a negative fd (a backend with no descriptor).
    ///
    /// # Safety
    /// `fd` must be a descriptor QEMU keeps open until the RAM block it backs is unregistered
    /// (`kf3_ram_del`), which removes every `BackendFd` of it from the device first.
    #[must_use]
    pub unsafe fn adopt(fd: i32) -> Option<BackendFd> {
        (fd >= 0).then_some(BackendFd(fd))
    }

    /// Borrow it for one `mmap`. ⊘ Never closed here.
    #[must_use]
    pub fn borrow(&self) -> std::os::fd::BorrowedFd<'_> {
        // SAFETY: `adopt`'s contract — the descriptor is open while a `BackendFd` of it exists in
        // the device, and `BorrowedFd` never closes it.
        unsafe { std::os::fd::BorrowedFd::borrow_raw(self.0) }
    }
}

/// ★ The C device's BAR1 overlay verb (`V3_BAR1_DOORBELL.md` §3.1): QUEUE change `seq` — `op` 1 =
/// show usermode-page bytes `[vf_rel, vf_rel+len)` write-trapped at BAR1 `[base, base+len)`; `op` 0
/// = remove the overlay at `base`. ⊘ Never waits (ruling 2026-09-26 (5)): it enqueues and schedules
/// a main-loop bottom half, which applies the changes in FIFO order under the BQL and reports each
/// through `kf3_bar1_overlay_done(seq, rc)`. Returns 0 once queued, or a negative errno. Called only
/// from the VA-manager thread.
pub type OverlayFn = unsafe extern "C" fn(
    opaque: *mut core::ffi::c_void,
    seq: u64,
    op: u32,
    base: u64,
    len: u64,
    vf_rel: u64,
) -> i32;

/// The registered overlay verb and its opaque device pointer.
#[derive(Debug, Clone, Copy)]
pub struct OverlayHook {
    f: OverlayFn,
    opaque: *mut core::ffi::c_void,
}

// SAFETY: `opaque` is the C device's state, which lives for the process (the device is never
// freed — `kf3_unrealize` only stops threads); the verb only takes the C queue's own mutex and
// schedules a bottom half, both thread-safe.
unsafe impl Send for OverlayHook {}
// SAFETY: as above.
unsafe impl Sync for OverlayHook {}

impl OverlayHook {
    /// Adopt the C device's verb.
    ///
    /// # Safety
    /// `f` must be callable from any non-vCPU thread with `opaque` for the process's lifetime, and
    /// must not block.
    #[must_use]
    pub unsafe fn adopt(f: OverlayFn, opaque: *mut core::ffi::c_void) -> OverlayHook {
        OverlayHook { f, opaque }
    }

    /// Queue change `seq`. `Err` carries the C device's negative errno (nothing was queued).
    ///
    /// # Errors
    /// The C device's refusal.
    pub fn submit(&self, seq: u64, op: u32, base: u64, len: u64, vf_rel: u64) -> Result<(), i32> {
        // SAFETY: the contract `adopt` was given.
        match unsafe { (self.f)(self.opaque, seq, op, base, len, vf_rel) } {
            0 => Ok(()),
            e => Err(e),
        }
    }
}

/// ★ 2026-09-30 — the C device's `KVM_IOEVENTFD` verb (`docs/design/V3_DOORBELL_IOEVENTFD.md`):
/// assign (`assign` = 1) or deassign one `len`-byte `DATAMATCH` MMIO ioeventfd for `datamatch` at
/// guest-physical `gpa`, signalling `fd`. Returns 0, or the kernel's negative errno. Thread-safe
/// (`kvm_vm_ioctl` takes no QEMU lock); called from the channel act thread and the main loop, never
/// from a vCPU or the register drainer.
pub type IoeventfdFn = unsafe extern "C" fn(
    opaque: *mut core::ffi::c_void,
    gpa: u64,
    len: u32,
    datamatch: u64,
    fd: i32,
    assign: u32,
) -> i32;

/// The registered ioeventfd verb and its opaque device pointer.
#[derive(Debug, Clone, Copy)]
pub struct IoeventfdHook {
    f: IoeventfdFn,
    opaque: *mut core::ffi::c_void,
}

// SAFETY: `opaque` is the C device's state, which lives for the process (the device is never freed —
// `kf3_unrealize` only stops threads); the verb makes one `ioctl` on QEMU's VM descriptor and takes
// no lock, so any thread may call it.
unsafe impl Send for IoeventfdHook {}
// SAFETY: as above.
unsafe impl Sync for IoeventfdHook {}

impl IoeventfdHook {
    /// Adopt the C device's verb.
    ///
    /// # Safety
    /// `f` must be callable from any non-vCPU thread with `opaque` for the process's lifetime, must
    /// only issue `KVM_IOEVENTFD` with the arguments given, and must not retain `fd`.
    #[must_use]
    pub unsafe fn adopt(f: IoeventfdFn, opaque: *mut core::ffi::c_void) -> IoeventfdHook {
        IoeventfdHook { f, opaque }
    }
}

impl kf_chan::dbfast::Ioeventfd for IoeventfdHook {
    fn set(
        &self,
        gpa: u64,
        value: u32,
        fd: std::os::fd::BorrowedFd<'_>,
        assign: bool,
    ) -> Result<(), i32> {
        use std::os::fd::AsRawFd;
        // SAFETY: the contract `adopt` was given; `fd` is a live borrowed descriptor for the whole
        // call, and the kernel takes its own reference to the eventfd behind it (never the number).
        match unsafe {
            (self.f)(
                self.opaque,
                gpa,
                4,
                u64::from(value),
                fd.as_raw_fd(),
                u32::from(assign),
            )
        } {
            0 => Ok(()),
            e => Err(e.saturating_neg()),
        }
    }
}

/// ★ 2026-10-03 (display step 3, `docs/design/V3_DISPLAY.md` §8) — the C device's fd-handler
/// verb for the broker relay: watch `fd` for readability (`read` = 1) and/or writability, or
/// (`read` = `write` = 0) remove the handler. Called only from inside a `kf3_broker_*` entry, so
/// on QEMU's main loop with the BQL held; always called with `(0, 0)` BEFORE Rust closes `fd`.
pub type BrokerWatchFn =
    unsafe extern "C" fn(opaque: *mut core::ffi::c_void, fd: i32, read: u32, write: u32);

/// ★ The C device's timer verb for the broker relay: fire at `deadline_ms`
/// (`QEMU_CLOCK_REALTIME`, the clock `kf3_broker_ready`'s `now_ms` reads), or never (`-1`).
/// Same thread and calling rules as [`BrokerWatchFn`].
pub type BrokerTimerFn = unsafe extern "C" fn(opaque: *mut core::ffi::c_void, deadline_ms: i64);

/// The C device's two broker verbs and its opaque state pointer.
#[derive(Debug, Clone, Copy)]
pub struct BrokerHooks {
    watch: BrokerWatchFn,
    timer: BrokerTimerFn,
    opaque: *mut core::ffi::c_void,
}

// SAFETY: `opaque` is the C device's state, which lives for the process; the hooks are only ever
// CALLED from inside a `kf3_broker_*` entry (the main loop) — the seat keeps them behind its
// mutex, and moving the pair between threads does nothing to the state it names.
unsafe impl Send for BrokerHooks {}

impl BrokerHooks {
    /// Adopt the C device's verbs.
    ///
    /// # Safety
    /// `watch` and `timer` must be callable with `opaque` on QEMU's main loop for the device's
    /// lifetime, must not re-enter any `kf3_broker_*` entry, and must not block.
    #[must_use]
    pub unsafe fn adopt(
        watch: BrokerWatchFn,
        timer: BrokerTimerFn,
        opaque: *mut core::ffi::c_void,
    ) -> BrokerHooks {
        BrokerHooks {
            watch,
            timer,
            opaque,
        }
    }
}

impl kf_broker::Host for BrokerHooks {
    fn watch(&mut self, fd: i32, read: bool, write: bool) {
        // SAFETY: the contract `adopt` was given; called from inside a `kf3_broker_*` entry.
        unsafe { (self.watch)(self.opaque, fd, u32::from(read), u32::from(write)) }
    }

    fn timer(&mut self, deadline_ms: Option<u64>) {
        let d = deadline_ms.map_or(-1, |t| i64::try_from(t).unwrap_or(i64::MAX));
        // SAFETY: as above.
        unsafe { (self.timer)(self.opaque, d) }
    }
}

/// Test-only owned bytes and permanently open fd. No unsafe adoption contract is
/// weakened for production; backing is intentionally retained until process exit.
#[cfg(test)]
pub(crate) fn test_owned_ram(bytes: usize) -> (RawRegion, BackendFd) {
    use std::os::fd::AsRawFd;
    let mem = Box::leak(vec![0u8; bytes].into_boxed_slice());
    let file = Box::leak(Box::new(std::fs::File::open("/dev/zero").unwrap()));
    (
        RawRegion {
            ptr: mem.as_mut_ptr(),
            len: mem.len(),
        },
        BackendFd(file.as_raw_fd()),
    )
}
