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
        if p.align_offset(core::mem::align_of::<u32>()) == 0 {
            // SAFETY: `span` bounds-checked 4 bytes and the pointer is 4-aligned; ONE volatile 32-bit
            // store, so a concurrent reader (a vCPU, or another thread's store of the same register —
            // the GSP heartbeat beside the drainer's publish) sees one value or the other, never a
            // mix.
            unsafe { core::ptr::write_volatile(p.cast::<u32>(), v) };
        } else {
            // SAFETY: `span` bounds-checked 4 bytes; unaligned-safe write of a plain integer.
            unsafe { core::ptr::write_unaligned(p.cast::<u32>(), v) };
        }
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
        if n == 4 {
            let b = v.to_le_bytes();
            return self.store_u32(off, u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
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

/// ★ DIAGNOSTIC (`KF3_BAR0_TRACE`, owner-approved 2026-10-07, `crate::bar0trace`): the C device's
/// BAR0 read-trap verb — `on` = 1 requests ROMD off on every shadow piece (their reads exit and are
/// answered from the same shadow), 0 restores ROMD. ⊘ Never waits: it stores the wish and schedules
/// a main-loop bottom half that applies it under the BQL. Called only from the register drainer.
pub type ReadTrapFn = unsafe extern "C" fn(opaque: *mut core::ffi::c_void, on: u32);

/// The registered read-trap verb and its opaque device pointer.
#[derive(Debug, Clone, Copy)]
pub struct ReadTrapHook {
    f: ReadTrapFn,
    opaque: *mut core::ffi::c_void,
}

// SAFETY: `opaque` is the C device's state, which lives for the process; the verb only stores an
// atomic and schedules a bottom half, both thread-safe.
unsafe impl Send for ReadTrapHook {}
// SAFETY: as above.
unsafe impl Sync for ReadTrapHook {}

impl ReadTrapHook {
    /// Adopt the C device's verb.
    ///
    /// # Safety
    /// `f` must be callable from any non-vCPU thread with `opaque` for the process's lifetime, and
    /// must not block.
    #[must_use]
    pub unsafe fn adopt(f: ReadTrapFn, opaque: *mut core::ffi::c_void) -> ReadTrapHook {
        ReadTrapHook { f, opaque }
    }

    /// Request the trap on or off.
    pub fn request(&self, on: bool) {
        // SAFETY: the contract `adopt` was given.
        unsafe { (self.f)(self.opaque, u32::from(on)) }
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

// ── ★ ABI 23 (2026-10-08, `OWNER_RULINGS.md` §V; `docs/design/V3_DISPLAY.md` §8.20): the C
// device's input and console-cursor verbs — `kf_broker::InputSink` and `kf_broker::CursorSink`
// for QEMU. Each is ONE QEMU call (or two: QEMU's wheel is a button press and release); every
// decision is kf-broker's. All are called only from inside a `kf3_broker_*` or
// `kf3_display_cursor_apply` entry, so on QEMU's main loop with the BQL held; none may block or
// re-enter an entry.

/// A key edge by Linux evdev code; 1 = QEMU mapped it, 0 = no mapping (dropped).
pub type InKeyFn =
    unsafe extern "C" fn(opaque: *mut core::ffi::c_void, evdev: u32, down: u32) -> i32;
/// A button edge: `button` is `KF3_BTN_*` (left 0, right 1, middle 2, side 3, extra 4);
/// `relative` = 1 for the relative pointing device, 0 for the absolute one.
pub type InButtonFn =
    unsafe extern "C" fn(opaque: *mut core::ffi::c_void, button: u32, down: u32, relative: u32);
/// One wheel detent (`dy` +1 up / -1 down, `dx` +1 right / -1 left), queued.
pub type InWheelFn =
    unsafe extern "C" fn(opaque: *mut core::ffi::c_void, dx: i32, dy: i32, relative: u32);
/// The absolute pointer at `(x, y)` in `0..width` x `0..height`, queued.
pub type InAbsFn =
    unsafe extern "C" fn(opaque: *mut core::ffi::c_void, x: u32, y: u32, width: u32, height: u32);
/// Relative motion, queued.
pub type InRelFn = unsafe extern "C" fn(opaque: *mut core::ffi::c_void, dx: i32, dy: i32);
/// End of one pointer report.
pub type InSyncFn = unsafe extern "C" fn(opaque: *mut core::ffi::c_void);
/// The guest's pointing devices into `out` (at most `cap`); returns how many.
pub type InPointersFn = unsafe extern "C" fn(
    opaque: *mut core::ffi::c_void,
    out: *mut crate::ffi_unsafe::Kf3Pointer,
    cap: u32,
) -> u32;
/// Make device `id` the one pointer events go to (`relative` names its kind).
pub type InSelectFn = unsafe extern "C" fn(opaque: *mut core::ffi::c_void, id: u32, relative: u32);
/// The guest has no device of this kind (once per kind per VM): say how to add one.
pub type InMissingFn = unsafe extern "C" fn(opaque: *mut core::ffi::c_void, relative: u32);
/// The user closed the window: `force` = 1 stop now, 0 an ACPI powerdown.
pub type InCloseFn = unsafe extern "C" fn(opaque: *mut core::ffi::c_void, force: u32);
/// The broker's window is `width` x `height` at `refresh_mhz` (0: unknown).
pub type InResizeFn =
    unsafe extern "C" fn(opaque: *mut core::ffi::c_void, width: u32, height: u32, refresh_mhz: u32);
/// The console's cursor image: `width` x `height` (1..=256 each), the hot spot inside, `pixels`
/// exactly `width * height` premultiplied `0xAARRGGBB` words valid for the call. 1 = defined.
pub type CurDefineFn = unsafe extern "C" fn(
    opaque: *mut core::ffi::c_void,
    width: u32,
    height: u32,
    hot_x: u32,
    hot_y: u32,
    pixels: *const u32,
) -> i32;
/// The console's hidden cursor. 1 = defined.
pub type CurHideFn = unsafe extern "C" fn(opaque: *mut core::ffi::c_void) -> i32;
/// The console's cursor hot spot to `(x, y)`. 1 = moved.
pub type CurMoveFn = unsafe extern "C" fn(opaque: *mut core::ffi::c_void, x: i32, y: i32) -> i32;
/// 1 when the console's pointer input is absolute now.
pub type CurAbsoluteFn = unsafe extern "C" fn(opaque: *mut core::ffi::c_void) -> u32;

/// ★ The C device's input and cursor verbs (every one present) and its opaque state.
#[derive(Debug, Clone, Copy)]
pub struct QemuSink {
    key: InKeyFn,
    button: InButtonFn,
    wheel: InWheelFn,
    abs: InAbsFn,
    rel: InRelFn,
    sync: InSyncFn,
    pointers: InPointersFn,
    select: InSelectFn,
    missing: InMissingFn,
    close: InCloseFn,
    resize: InResizeFn,
    cur_define: CurDefineFn,
    cur_hide: CurHideFn,
    cur_move: CurMoveFn,
    cur_absolute: CurAbsoluteFn,
    opaque: *mut core::ffi::c_void,
}

// SAFETY: as [`BrokerHooks`] — `opaque` is the C device's state, which lives for the process, and
// the verbs are only CALLED on the main loop from inside an entry; the seat keeps the copy behind
// a mutex.
unsafe impl Send for QemuSink {}

impl QemuSink {
    /// Adopt the C device's verbs; `None` when any is missing.
    ///
    /// # Safety
    /// Every verb in `ops` must be callable with `opaque` on QEMU's main loop for the device's
    /// lifetime, must not re-enter any `kf3_*` entry, and must not block; the pointers verb writes
    /// at most `cap` entries, the define verb reads at most `width * height` words.
    #[must_use]
    pub unsafe fn adopt(
        ops: &crate::ffi_unsafe::Kf3InputOps,
        opaque: *mut core::ffi::c_void,
    ) -> Option<QemuSink> {
        Some(QemuSink {
            key: ops.key?,
            button: ops.button?,
            wheel: ops.wheel?,
            abs: ops.abs?,
            rel: ops.rel?,
            sync: ops.sync?,
            pointers: ops.pointers?,
            select: ops.select_pointer?,
            missing: ops.missing_pointer?,
            close: ops.close?,
            resize: ops.resize_hint?,
            cur_define: ops.cursor_define?,
            cur_hide: ops.cursor_hide?,
            cur_move: ops.cursor_move?,
            cur_absolute: ops.cursor_absolute?,
            opaque,
        })
    }
}

/// The `KF3_BTN_*` number of a forwarded button (`wire_mirror.rs` checks it against kf3.h).
#[must_use]
pub fn button_abi(b: kf_broker::Button) -> u32 {
    match b {
        kf_broker::Button::Left => 0,
        kf_broker::Button::Right => 1,
        kf_broker::Button::Middle => 2,
        kf_broker::Button::Side => 3,
        kf_broker::Button::Extra => 4,
    }
}

fn rel_abi(p: kf_broker::Pointer) -> u32 {
    u32::from(p == kf_broker::Pointer::Relative)
}

impl kf_broker::InputSink for QemuSink {
    fn key(&mut self, code: u16, down: bool) -> bool {
        // SAFETY: the contract `adopt` was given (every call below too); called from inside an
        // entry.
        unsafe { (self.key)(self.opaque, u32::from(code), u32::from(down)) == 1 }
    }
    fn button(&mut self, b: kf_broker::Button, down: bool, to: kf_broker::Pointer) {
        // SAFETY: as above.
        unsafe { (self.button)(self.opaque, button_abi(b), u32::from(down), rel_abi(to)) }
    }
    fn wheel(&mut self, dx: i32, dy: i32, to: kf_broker::Pointer) {
        // SAFETY: as above.
        unsafe { (self.wheel)(self.opaque, dx, dy, rel_abi(to)) }
    }
    fn abs(&mut self, x: u32, y: u32, r: kf_broker::AbsRange) {
        // SAFETY: as above.
        unsafe { (self.abs)(self.opaque, x, y, r.width, r.height) }
    }
    fn rel(&mut self, dx: i32, dy: i32) {
        // SAFETY: as above.
        unsafe { (self.rel)(self.opaque, dx, dy) }
    }
    fn sync(&mut self) {
        // SAFETY: as above.
        unsafe { (self.sync)(self.opaque) }
    }
    fn pointer_devices(&mut self, out: &mut [kf_broker::PointerDevice]) -> usize {
        let mut raw =
            [crate::ffi_unsafe::Kf3Pointer::default(); kf_broker::input::MAX_POINTER_DEVICES];
        let cap = raw.len().min(out.len());
        // SAFETY: as above; `raw` is writable for `cap` entries, which the verb does not exceed.
        let n = unsafe {
            (self.pointers)(
                self.opaque,
                raw.as_mut_ptr(),
                u32::try_from(cap).unwrap_or(0),
            )
        };
        let n = (n as usize).min(cap);
        for (o, r) in out.iter_mut().zip(&raw[..n]) {
            *o = r.device();
        }
        n
    }
    fn select_pointer(&mut self, id: u32, kind: kf_broker::Pointer) {
        // SAFETY: as above.
        unsafe { (self.select)(self.opaque, id, rel_abi(kind)) }
    }
    fn missing_pointer(&mut self, kind: kf_broker::Pointer) {
        // SAFETY: as above.
        unsafe { (self.missing)(self.opaque, rel_abi(kind)) }
    }
    fn close(&mut self, r: kf_broker::PowerRequest) {
        let force = u32::from(r == kf_broker::PowerRequest::ForceOff);
        // SAFETY: as above.
        unsafe { (self.close)(self.opaque, force) }
    }
    fn resize_hint(&mut self, width: u32, height: u32, refresh_mhz: u32) {
        // SAFETY: as above.
        unsafe { (self.resize)(self.opaque, width, height, refresh_mhz) }
    }
}

impl kf_broker::CursorSink for QemuSink {
    fn define_cursor(&mut self, s: kf_broker::CursorShape, pixels: &[u32]) -> bool {
        if pixels.len() != s.width as usize * s.height as usize {
            return false;
        }
        // SAFETY: as above; `pixels` holds exactly `width * height` words for the call.
        unsafe {
            (self.cur_define)(
                self.opaque,
                s.width,
                s.height,
                s.hot.0,
                s.hot.1,
                pixels.as_ptr(),
            ) == 1
        }
    }
    fn hide_cursor(&mut self) -> bool {
        // SAFETY: as above.
        unsafe { (self.cur_hide)(self.opaque) == 1 }
    }
    fn move_cursor(&mut self, x: i32, y: i32) -> bool {
        // SAFETY: as above.
        unsafe { (self.cur_move)(self.opaque, x, y) == 1 }
    }
    fn pointer_is_absolute(&mut self) -> bool {
        // SAFETY: as above.
        unsafe { (self.cur_absolute)(self.opaque) != 0 }
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

/// ★ ABI 23 (`V3_DISPLAY.md` §8.20): the QEMU sink's own conversions, against C-ABI verbs that
/// record what they are called with (no QEMU): button numbers, the relative flag, a device list
/// the VMM over-reports and whose names fill the field without a NUL, the define verb's pixels.
#[cfg(test)]
mod qemu_sink_tests {
    use super::*;
    use crate::ffi_unsafe::{Kf3InputOps, Kf3Pointer};
    use core::ffi::c_void;
    use std::cell::RefCell;

    thread_local! {
        static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    fn say(s: String) {
        LOG.with(|l| l.borrow_mut().push(s));
    }
    fn said() -> Vec<String> {
        LOG.with(|l| std::mem::take(&mut *l.borrow_mut()))
    }
    // safe `extern "C"` items: a safe function pointer coerces to the verbs' `unsafe` types
    extern "C" fn key(_: *mut c_void, c: u32, d: u32) -> i32 {
        say(format!("key {c} {d}"));
        i32::from(c != 0x2fe)
    }
    extern "C" fn button(_: *mut c_void, b: u32, d: u32, r: u32) {
        say(format!("button {b} {d} rel={r}"));
    }
    extern "C" fn wheel(_: *mut c_void, dx: i32, dy: i32, r: u32) {
        say(format!("wheel {dx} {dy} rel={r}"));
    }
    extern "C" fn abs(_: *mut c_void, x: u32, y: u32, w: u32, h: u32) {
        say(format!("abs {x} {y} {w}x{h}"));
    }
    extern "C" fn rel(_: *mut c_void, dx: i32, dy: i32) {
        say(format!("rel {dx} {dy}"));
    }
    extern "C" fn sync(_: *mut c_void) {
        say("sync".into());
    }
    extern "C" fn pointers(_: *mut c_void, out: *mut Kf3Pointer, cap: u32) -> u32 {
        say(format!("pointers cap={cap}"));
        let n = cap.min(3) as usize;
        // SAFETY: the sink passes `out` writable for `cap` entries (the verb's contract).
        let out = unsafe { core::slice::from_raw_parts_mut(out, n) };
        for (i, o) in (0u32..).zip(out.iter_mut()) {
            *o = Kf3Pointer {
                id: 10 + i,
                absolute: u8::from(i == 1),
                paravirtual: u8::from(i > 0),
                pad: [0; 2],
                name: [b'M'; 56], // no NUL: the whole field is the name
            };
        }
        cap + 5 // over-reports: the sink must not trust it
    }
    extern "C" fn select(_: *mut c_void, id: u32, r: u32) {
        say(format!("select {id} rel={r}"));
    }
    extern "C" fn missing(_: *mut c_void, r: u32) {
        say(format!("missing rel={r}"));
    }
    extern "C" fn close(_: *mut c_void, f: u32) {
        say(format!("close {f}"));
    }
    extern "C" fn resize(_: *mut c_void, w: u32, h: u32, m: u32) {
        say(format!("resize {w} {h} {m}"));
    }
    extern "C" fn cdefine(_: *mut c_void, w: u32, h: u32, hx: u32, hy: u32, px: *const u32) -> i32 {
        // SAFETY: the sink passes `px` holding `w * h` words for the call (the verb's contract).
        let px = unsafe { core::slice::from_raw_parts(px, (w * h) as usize) };
        say(format!(
            "define {w}x{h} {hx},{hy} last={:#x}",
            px[px.len() - 1]
        ));
        1
    }
    extern "C" fn chide(_: *mut c_void) -> i32 {
        say("hide".into());
        1
    }
    extern "C" fn cmove(_: *mut c_void, x: i32, y: i32) -> i32 {
        say(format!("move {x} {y}"));
        1
    }
    extern "C" fn cabs(_: *mut c_void) -> u32 {
        1
    }

    fn ops() -> Kf3InputOps {
        Kf3InputOps {
            key: Some(key),
            button: Some(button),
            wheel: Some(wheel),
            abs: Some(abs),
            rel: Some(rel),
            sync: Some(sync),
            pointers: Some(pointers),
            select_pointer: Some(select),
            missing_pointer: Some(missing),
            close: Some(close),
            resize_hint: Some(resize),
            cursor_define: Some(cdefine),
            cursor_hide: Some(chide),
            cursor_move: Some(cmove),
            cursor_absolute: Some(cabs),
        }
    }

    #[test]
    fn a_missing_verb_is_refused() {
        let mut o = ops();
        o.cursor_absolute = None;
        // SAFETY: the verbs above honour the contract; `opaque` is never used (refused first).
        assert!(unsafe { QemuSink::adopt(&o, core::ptr::null_mut()) }.is_none());
    }

    #[test]
    fn the_policy_reaches_the_c_verbs_with_the_abi_numbers() {
        use kf_broker::{Input, InputPolicy, Pointer};
        // SAFETY: the verbs above honour the contract and never read `opaque`.
        let mut s = unsafe { QemuSink::adopt(&ops(), core::ptr::null_mut()) }.unwrap();
        said();
        let mut p = InputPolicy::new();
        p.connected(&mut s);
        p.deliver(
            &[
                Input::Grab(true),
                Input::Btn {
                    code: 0x114,
                    down: true,
                    to: Pointer::Relative,
                },
                Input::Wheel {
                    dx: 0,
                    dy: -1,
                    to: Pointer::Relative,
                },
                Input::Rel { dx: 3, dy: -4 },
                Input::Grab(false),
                Input::Abs {
                    x: 5,
                    y: 6,
                    w: 640,
                    h: 480,
                },
                Input::Key {
                    code: 0x2fe,
                    down: true,
                },
                Input::Close { force: true },
                Input::Surface {
                    w: 800,
                    h: 600,
                    mhz: 60_000,
                },
            ],
            &mut s,
        );
        assert_eq!(
            said(),
            [
                "pointers cap=16",
                "pointers cap=16",
                "select 12 rel=1",
                "button 4 1 rel=1",
                "sync",
                "wheel 0 -1 rel=1",
                "sync",
                "rel 3 -4",
                "sync",
                "pointers cap=16",
                "select 11 rel=0",
                "abs 5 6 640x480",
                "sync",
                "key 766 1",
                "close 1",
                "resize 800 600 60000",
            ]
        );
        assert_eq!(p.counters().keys_unmapped, 1);
        // the device list: the verb's count (9) is cut to what was asked for (4; the entry it did
        // not write is the zeroed default), names cut at the field
        let mut out: [kf_broker::PointerDevice; 4] = Default::default();
        let n = kf_broker::InputSink::pointer_devices(&mut s, &mut out);
        assert_eq!(n, 4);
        assert_eq!((out[3].id, out[3].name.as_str()), (0, ""));
        assert_eq!(out[2].name.len(), 56);
        assert!(out[1].kind == Pointer::Absolute && out[1].paravirtual);
        // the cursor: the define carries exactly width*height words
        said();
        let shape = kf_broker::CursorShape {
            width: 2,
            height: 3,
            hot: (1, 2),
        };
        let px = [1, 2, 3, 4, 5, 0xdead_beef];
        assert!(kf_broker::CursorSink::define_cursor(&mut s, shape, &px));
        assert!(!kf_broker::CursorSink::define_cursor(
            &mut s,
            shape,
            &px[..5]
        ));
        assert_eq!(said(), ["define 2x3 1,2 last=0xdeadbeef"]);
    }
}
