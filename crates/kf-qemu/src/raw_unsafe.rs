//! ★ The ONE raw-memory type: a region QEMU owns (BAR0 shadow RAM, guest RAM) that Rust reads and
//! writes. Everything else in this crate is safe code over it.

/// `[ptr, ptr+len)` of memory QEMU allocated and keeps mapped while the region is registered.
///
/// ★ Security fix C-02 (review 2, 2026-11-13): **not `Clone`, not `Copy`.** A guest-RAM region
/// is owned by exactly one [`crate::mem::RamMap`] entry from `kf3_ram_add` to `kf3_ram_del`, and
/// safe code reaches it only through a guard that holds the map's read lock across one bounded
/// copy (`RamMap::block_for` / `RamMap::read_file` and friends). ⊘ The text this corrects: *"a
/// `Copy` raw region"* — copies left the guard and were used after `kf3_ram_del` let QEMU free the
/// block (use-after-free of VMM memory, steered by guest ring contents). With no copy, the only
/// way to keep the address past the unplug is to keep the guard, and the unplug's write lock
/// waits for it.
#[derive(Debug)]
pub struct RawRegion {
    ptr: *mut u8,
    len: usize,
}

// SAFETY: the region is process memory QEMU keeps mapped from registration until the device (or
// the RAM block) is torn down, which unregisters it here first. Concurrent access is the device's
// ordinary model (a guest vCPU and our threads touch the same page, as hardware DMA does); every
// access below is a bounds-checked volatile or byte copy, never a reference. The type is neither
// `Clone` nor `Copy`, so the one owner (the RAM map entry or the device's shadow piece) is the
// only holder, and unregistering removes it.
unsafe impl Send for RawRegion {}
// SAFETY: as above — no `&`/`&mut` into the memory is ever created, only volatile/copy access.
unsafe impl Sync for RawRegion {}

/// ★ C-02 (2026-11-13): a compile-time guard that `RawRegion` and [`BackendFd`] are never made
/// `Copy` again (the trait resolution below becomes ambiguous — a build error — for a `Copy`
/// type). ⊘ `BackendFd` is `Clone` on purpose: a clone shares the owned descriptor.
const _: fn() = || {
    trait AmbiguousIfCopy<A> {
        fn some_item() {}
    }
    impl<T: ?Sized> AmbiguousIfCopy<()> for T {}
    impl<T: Copy> AmbiguousIfCopy<u8> for T {}
    let _ = <RawRegion as AmbiguousIfCopy<_>>::some_item;
    let _ = <BackendFd as AmbiguousIfCopy<_>>::some_item;
};

impl RawRegion {
    /// Adopt `[ptr, ptr+len)`.
    ///
    /// What CAN be checked is checked here, once, and panics (a wrong caller stops the VMM, never
    /// corrupts it): a null `ptr` with a non-zero `len`, and a range that wraps the address space.
    ///
    /// CALLER-CONTRACT: that the range is mapped, writable and stays so until this region is
    /// dropped. Not validated because no syscall can prove QEMU will keep it mapped; if it breaks,
    /// a copy touches unmapped or reused memory. The one production caller is `kf3_ram_add` /
    /// `kf3_shadow_attach` (QEMU's RAM block / shadow allocation), and the ONE owner of the result
    /// is the RAM map entry or the shadow piece, removed before QEMU frees the memory (C-02).
    ///
    /// # Safety
    /// `ptr` must be valid for reads and writes of `len` bytes until this region is dropped from
    /// every structure holding it (the device unregisters it before QEMU frees the memory).
    ///
    /// # Panics
    /// On a null `ptr` with `len > 0`, or when `ptr + len` overflows the address space.
    #[must_use]
    pub unsafe fn adopt(ptr: *mut u8, len: usize) -> RawRegion {
        assert!(
            len == 0 || !ptr.is_null(),
            "RawRegion::adopt: null pointer for {len} bytes"
        );
        assert!(
            (ptr as usize).checked_add(len).is_some(),
            "RawRegion::adopt: [{ptr:p}, +{len:#x}) wraps the address space"
        );
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
///
/// ★ Security fix C-03 / A-01 (review 2, 2026-11-13): the token OWNS a duplicate of the
/// backend's descriptor (`F_DUPFD_CLOEXEC` at adoption, shared by `Arc`), so [`Self::borrow`]
/// lends a descriptor that is open for as long as the token (or a clone of it) lives — whatever
/// QEMU does with its own number afterwards. ⊘ The text this corrects: *"a `Copy` integer"* whose
/// `borrow()` fabricated a `BorrowedFd`: a backend closed between the lookup and the VA thread's
/// `mmap(MAP_FIXED)` let a REUSED number (a disk image, our own memfd) be mapped into a
/// guest-visible BAR window. Equality is the FILE's identity (`st_dev`, `st_ino`), which is what
/// "the one fd every block shares" always meant: QEMU's two q35 aliases of `ram0` hand over the
/// same file, and two different backends are never equal even if QEMU reused a number.
#[derive(Debug, Clone)]
pub struct BackendFd {
    file: std::sync::Arc<std::fs::File>,
    dev: u64,
    ino: u64,
}

impl PartialEq for BackendFd {
    fn eq(&self, other: &Self) -> bool {
        self.dev == other.dev && self.ino == other.ino
    }
}
impl Eq for BackendFd {}

impl BackendFd {
    /// Adopt `fd` by duplicating it; `None` for a negative fd (a backend with no descriptor), or
    /// when the duplicate or its `fstat` is refused (reported on stderr: the block then behaves as
    /// a backend without a descriptor, exactly as before for `fd < 0`).
    ///
    /// CALLER-CONTRACT: that `fd` is the memory backend's descriptor for the block being
    /// registered. Not validated because any file can back guest RAM (memfd, hugetlbfs, a plain
    /// file); if it breaks, BAR windows show that file instead of guest RAM. A closed number is
    /// refused (the duplicate fails), and what is adopted can no longer be swapped by a close and
    /// a reuse of the number.
    ///
    /// # Safety
    /// `fd` must be an open descriptor for the duration of this call. Nothing is assumed about it
    /// afterwards: the token holds its own duplicate.
    #[must_use]
    pub unsafe fn adopt(fd: i32) -> Option<BackendFd> {
        if fd < 0 {
            return None;
        }
        // SAFETY: the caller promised `fd` is open for this call; the borrow does not outlive it
        // (it is duplicated on the next line and dropped at the end of the statement).
        let owned = unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) }
            .try_clone_to_owned()
            .map_err(|e| eprintln!("kf3: backend fd {fd}: dup refused ({e})"))
            .ok()?;
        Self::own(std::fs::File::from(owned))
    }

    /// Take ownership of an open file (identity read once, here).
    fn own(file: std::fs::File) -> Option<BackendFd> {
        use std::os::unix::fs::MetadataExt;
        let meta = file
            .metadata()
            .map_err(|e| eprintln!("kf3: backend fd: fstat refused ({e})"))
            .ok()?;
        Some(BackendFd {
            dev: meta.dev(),
            ino: meta.ino(),
            file: std::sync::Arc::new(file),
        })
    }

    /// Borrow it for one `mmap`. ⊘ Never closed here; open for as long as `self` lives.
    #[must_use]
    pub fn borrow(&self) -> std::os::fd::BorrowedFd<'_> {
        use std::os::fd::AsFd;
        self.file.as_fd()
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
    /// CALLER-CONTRACT: the `# Safety` text below — a C function pointer and its opaque state
    /// cannot be validated from Rust; if it breaks, the call jumps to or passes garbage.
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
    /// CALLER-CONTRACT: the `# Safety` text below — a C function pointer and its opaque state
    /// cannot be validated from Rust; if it breaks, the call jumps to or passes garbage.
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
    /// CALLER-CONTRACT: the `# Safety` text below — a C function pointer and its opaque state
    /// cannot be validated from Rust; if it breaks, the call jumps to or passes garbage.
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
    /// CALLER-CONTRACT: the `# Safety` text below — C function pointers and their opaque state
    /// cannot be validated from Rust; if it breaks, a call jumps to or passes garbage.
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
    /// CALLER-CONTRACT: the `# Safety` text below — C function pointers and their opaque state
    /// cannot be validated from Rust; if it breaks, a call jumps to or passes garbage.
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

/// Test-only owned bytes and an open fd (a fresh memfd, so two calls are two distinct files). No
/// unsafe adoption contract is weakened for production; the bytes are intentionally retained until
/// process exit.
#[cfg(test)]
pub(crate) fn test_owned_ram(bytes: usize) -> (RawRegion, BackendFd) {
    let mem = Box::leak(vec![0u8; bytes].into_boxed_slice());
    let file = kf_linux_raw::SharedRam::create_named(c"kf-test-ram", 4096)
        .unwrap()
        .dup_for_export()
        .unwrap();
    (
        RawRegion {
            ptr: mem.as_mut_ptr(),
            len: mem.len(),
        },
        BackendFd::own(std::fs::File::from(file)).unwrap(),
    )
}

#[cfg(test)]
impl RawRegion {
    /// Test-only: a second handle on memory [`test_owned_ram`] leaked for the process's life (so
    /// no lifetime can be violated). ⊘ Not available outside `cfg(test)`: production regions have
    /// one owner.
    pub(crate) fn test_alias(&self) -> RawRegion {
        RawRegion {
            ptr: self.ptr,
            len: self.len,
        }
    }
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

/// ★ Security fixes C-02 / C-03 (review 2, 2026-11-13): the two adoption doors called with
/// hostile or wrong arguments refuse (or panic), and an adopted descriptor cannot be swapped for
/// another file by closing and reusing its number.
#[cfg(test)]
mod adoption_tests {
    use super::*;
    use std::io::{Seek, SeekFrom, Write};
    use std::os::fd::AsRawFd;

    /// The ONE door these tests open `RawRegion::adopt` through — with deliberately wrong
    /// arguments; adoption must panic or yield a region that refuses every access.
    fn adopt_raw(ptr: usize, len: usize) -> RawRegion {
        // SAFETY: test-only. No region adopted here is ever accessed inside its range: each test
        // either expects the panic or asserts the access is refused by the bounds check first.
        unsafe { RawRegion::adopt(ptr as *mut u8, len) }
    }

    /// The ONE door these tests open `BackendFd::adopt` through.
    fn adopt_fd(fd: i32) -> Option<BackendFd> {
        // SAFETY: test-only. Negative and closed numbers are deliberate contract violations the
        // door must refuse; an open number is open for the call.
        unsafe { BackendFd::adopt(fd) }
    }

    fn memfd_with(tag: &[u8]) -> std::fs::File {
        let fd = kf_linux_raw::SharedRam::create_named(c"kf-adopt-test", 4096)
            .unwrap()
            .dup_for_export()
            .unwrap();
        let mut f = std::fs::File::from(fd);
        f.write_all(tag).unwrap();
        f
    }

    fn tag_of(fd: std::os::fd::BorrowedFd<'_>) -> Vec<u8> {
        use std::io::Read;
        let mut f = std::fs::File::from(fd.try_clone_to_owned().unwrap());
        f.seek(SeekFrom::Start(0)).unwrap();
        let mut b = [0u8; 4];
        f.read_exact(&mut b).unwrap();
        b.to_vec()
    }

    /// RED on the pre-fix tree (`BackendFd` was the bare number: after QEMU closed it and the
    /// number was reused, `borrow()` lent the NEW file — which the VA thread then `mmap`ed into a
    /// guest-visible BAR window). GREEN: the token owns a duplicate of the file it adopted.
    #[test]
    fn a_closed_and_reused_backend_number_still_lends_the_adopted_file() {
        let backend = memfd_with(b"RAM0");
        let raw = backend.as_raw_fd();
        let token = adopt_fd(raw).expect("adopted");
        // QEMU closes the backend; a disk image opens and gets the same number.
        drop(backend);
        let intruder = memfd_with(b"DISK");
        let reused = intruder.as_raw_fd() == raw;
        assert_eq!(
            tag_of(token.borrow()),
            b"RAM0",
            "the backend token lends another file (number reused: {reused})"
        );
        let other = BackendFd::own(intruder.try_clone().unwrap()).unwrap();
        assert_ne!(
            token, other,
            "two different files are never the same backend"
        );
    }

    #[test]
    fn hostile_descriptor_numbers_are_refused() {
        for fd in [-1, i32::MIN, -4096] {
            assert!(adopt_fd(fd).is_none(), "{fd}");
        }
        // A number that is not open: the duplicate is refused, nothing is adopted.
        assert!(adopt_fd(i32::MAX).is_none());
    }

    #[test]
    fn two_adoptions_of_one_file_are_the_same_backend_and_outlive_the_original() {
        let backend = memfd_with(b"RAM1");
        let (a, b) = (
            adopt_fd(backend.as_raw_fd()).unwrap(),
            adopt_fd(backend.as_raw_fd()).unwrap(),
        );
        assert_eq!(a, b, "q35's two aliases of one memfd are one backend");
        drop(backend);
        let c = a.clone();
        drop(a);
        assert_eq!(tag_of(c.borrow()), b"RAM1");
        assert_eq!(tag_of(b.borrow()), b"RAM1");
    }

    #[test]
    #[should_panic(expected = "null pointer")]
    fn a_null_region_with_a_length_panics() {
        let _ = adopt_raw(0, 16);
    }

    #[test]
    #[should_panic(expected = "null pointer")]
    fn a_null_region_of_one_byte_panics() {
        let _ = adopt_raw(0, 1);
    }

    #[test]
    #[should_panic(expected = "wraps the address space")]
    fn a_region_that_wraps_the_address_space_panics() {
        let _ = adopt_raw(usize::MAX, 2);
    }

    #[test]
    #[should_panic(expected = "wraps the address space")]
    fn a_region_ending_one_past_the_address_space_panics() {
        let _ = adopt_raw(1usize << 63, 1usize << 63);
    }

    #[test]
    fn a_region_ending_exactly_at_the_top_is_accepted_and_never_touched() {
        let r = adopt_raw(usize::MAX - 3, 3);
        assert_eq!(r.len(), 3);
        assert!(r.load_u32(0).is_none(), "4 bytes do not fit in 3");
    }

    #[test]
    fn a_backend_without_a_descriptor_is_none() {
        // QEMU passes -1 for a backend with no fd; `BorrowedFd` may never hold -1.
        assert!(adopt_fd(-1).is_none());
    }

    #[test]
    fn an_empty_region_refuses_every_access() {
        // A null pointer is allowed only for an empty region.
        let r = adopt_raw(0, 0);
        assert!(r.is_empty());
        let mut b = [0u8; 1];
        assert!(!r.read_into(0, &mut b));
        assert!(!r.write_from(0, &b));
        assert!(r.load_u32(0).is_none());
        assert!(!r.store_u32(0, 1));
        assert!(r.read_into(0, &mut []));
    }

    #[test]
    fn hostile_offsets_into_a_region_are_refused() {
        let (r, _) = test_owned_ram(16);
        let mut b = [0u8; 4];
        assert!(!r.read_into(usize::MAX, &mut b));
        assert!(!r.read_into(13, &mut b));
        assert!(r.read_into(12, &mut b));
        assert!(r.load_u32(usize::MAX - 1).is_none());
        assert!(!r.store(0, 0, 3), "width 3 is refused");
        assert!(!r.store(12, 0, 8), "straddles the end");
        assert!(r.store(8, 0x0102_0304_0506_0708, 8));
        assert_eq!(r.load_u32(8), Some(0x0506_0708));
    }
}
