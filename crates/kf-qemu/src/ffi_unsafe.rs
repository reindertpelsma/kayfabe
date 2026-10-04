//! ★ The `extern "C"` surface the kf3-gpu QOM device calls. Every entry point validates its handle
//! and pointers and hands off to the safe [`crate::device::Device`] immediately.

// ★ G1d (v3-sec-rawaddr, scripts/ci/address_clippy.sh): this file is the memory-safety perimeter
// (OWNER_RULINGS §R), the one place the pointer functions and types Clippy is told to refuse may
// appear. The opt-out is refused anywhere else (scripts/ci/address_gate.py).
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use crate::device::{Config, Device};
use crate::raw_unsafe::RawRegion;
use core::ffi::{c_char, c_void};
use std::ffi::CStr;

/// Wire ABI of this surface; the C device refuses a mismatched archive.
/// ★ 10 (2026-09-30, `v3-mc22`): the union of two INDEPENDENT 9s — `v3-display2`'s frame hand-off
/// ([`Kf3Frame`], [`kf3_display_frame`]) and `v3-ioeventfd`'s doorbell fast path
/// ([`kf3_doorbell_page_offset`], [`kf3_set_ioeventfd`], [`kf3_doorbell_site`]). The two 9s name
/// different surfaces, so an archive from either branch must fail the device's check: one new
/// number above both. `tests/wire_mirror.rs` compiles every entry point here against `kf3.h`.
/// ★ 11 (2026-10-03, `v3-gop-kf3`, `docs/design/V3_DISPLAY.md` §4.11): the boot display —
/// [`kf3_realize`] takes `gop`, and [`kf3_option_rom`] hands the C device the ROM to register.
/// ★ 14 (2026-10-04, `v3-sec-rawaddr`, audit S1-03): [`Kf3Frame`] carries the frame's `len`, and
/// [`kf3_display_frame`] answers `-2` for a frame it refused ([`check_frame`]). ⊘ 12 is
/// `v3-broker`'s and 13 `v3-dispsw-exp`'s: whichever branch merges second takes the maximum plus one
/// AT MERGE TIME and records every claim here and in `kf3.h`.
pub const KF3_ABI: u32 = 14;

/// The PCI identity the C device presents.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct Kf3Identity {
    /// Vendor id.
    pub vendor: u16,
    /// Device id.
    pub device: u16,
    /// Subsystem vendor id.
    pub subsystem_vendor: u16,
    /// Subsystem id.
    pub subsystem: u16,
    /// Class code (24 bits).
    pub class: u32,
    /// Revision id.
    pub revision: u8,
    /// Padding.
    pub pad: [u8; 3],
    /// BAR0 bytes.
    pub bar0_bytes: u64,
}

/// One memory-map region: `how` = 0 plain RAM, 1 shadow + write trap, 2 host passthrough, 3 hole.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct Kf3Region {
    /// PCI BAR index as the guest numbers it (0, 1, 2 = RM's BAR2).
    pub bar: u8,
    /// Disposition.
    pub how: u8,
    /// Padding.
    pub pad: [u8; 6],
    /// Offset inside the BAR.
    pub base: u64,
    /// Length.
    pub len: u64,
}

fn dev<'a>(h: *mut c_void) -> Option<&'a Device> {
    // SAFETY: a non-null handle is only ever a pointer `kf3_realize` produced from `Box::into_raw`
    // and that `kf3_unrealize` has not yet freed (the C device drops it exactly once, at unrealize).
    (!h.is_null()).then(|| unsafe { &*h.cast::<Device>() })
}

fn write_err(buf: *mut c_char, len: usize, msg: &str) {
    if buf.is_null() || len == 0 {
        return;
    }
    let n = msg.len().min(len - 1);
    // SAFETY: the caller passed a writable buffer of `len` bytes; we write `n + 1 <= len`.
    unsafe {
        core::ptr::copy_nonoverlapping(msg.as_ptr(), buf.cast::<u8>(), n);
        *buf.add(n) = 0;
    }
}

/// The archive's ABI.
#[unsafe(no_mangle)]
pub extern "C" fn kf3_abi_version() -> u32 {
    KF3_ABI
}

/// Realize the device and start its register drainer. Returns 0 and a handle, or -1 with a message.
///
/// # Safety
/// `guest_driver` is null or a NUL-terminated string; `out` is writable; `err` is null or writable
/// for `err_len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kf3_realize(
    gpu_minor: u32,
    fb_mb: u64,
    bar1_bytes: u64,
    bar2_bytes: u64,
    guest_driver: *const c_char,
    display: u32,
    gop: u32,
    out: *mut *mut c_void,
    err: *mut c_char,
    err_len: usize,
) -> i32 {
    let guest = if guest_driver.is_null() {
        None
    } else {
        // SAFETY: the caller promises a NUL-terminated string.
        Some(
            unsafe { CStr::from_ptr(guest_driver) }
                .to_string_lossy()
                .into_owned(),
        )
        .filter(|s| !s.is_empty())
    };
    let cfg = Config {
        gpu_minor,
        fb_mb,
        bar1_bytes,
        bar2_bytes,
        guest_driver: guest,
        display: display != 0,
        gop: gop != 0,
    };
    match Device::realize(&cfg) {
        Ok(d) => {
            let d: &'static Device = Box::leak(Box::new(d));
            if std::thread::Builder::new()
                .name("kf3-drainer".into())
                .spawn(move || d.drainer_loop())
                .is_err()
            {
                write_err(err, err_len, "could not start the register drainer thread");
                return -1;
            }
            // ★ P5: the workers — they serve rung Translated tokens and host completions.
            for i in 0..2 {
                if std::thread::Builder::new()
                    .name(format!("kf3-worker{i}"))
                    .spawn(move || d.worker_loop())
                    .is_err()
                {
                    write_err(err, err_len, "could not start a worker thread");
                    return -1;
                }
            }
            // ★ v3-initrace: the completion probe (`KF3_COMPLETION_PROBE`, default off).
            if crate::chan::completion_probe_ms().is_some()
                && std::thread::Builder::new()
                    .name("kf3-probe".into())
                    .spawn(move || d.probe_loop())
                    .is_err()
            {
                write_err(err, err_len, "could not start the completion-probe thread");
                return -1;
            }
            // ★ v3-display: the display worker (`display=on` only).
            if d.display.is_some()
                && std::thread::Builder::new()
                    .name("kf3-display".into())
                    .spawn(move || d.display_loop())
                    .is_err()
            {
                write_err(err, err_len, "could not start the display worker thread");
                return -1;
            }
            // ★ P4: the VA-manager thread — the one owner of the GPU walker.
            if std::thread::Builder::new()
                .name("kf3-vamgr".into())
                .spawn(move || d.va_loop())
                .is_err()
            {
                write_err(err, err_len, "could not start the VA-manager thread");
                return -1;
            }
            if !out.is_null() {
                // SAFETY: `out` is writable (caller contract).
                unsafe { *out = (d as *const Device).cast_mut().cast::<c_void>() };
            }
            0
        }
        Err(e) => {
            write_err(err, err_len, &e);
            -1
        }
    }
}

/// Fill `out` with the PCI identity.
///
/// # Safety
/// `out` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kf3_identity(h: *mut c_void, out: *mut Kf3Identity) -> i32 {
    let (Some(d), false) = (dev(h), out.is_null()) else {
        return -1;
    };
    let p = d.identity.pci;
    let id = Kf3Identity {
        vendor: p.vendor,
        device: p.device,
        subsystem_vendor: p.subsystem_vendor,
        subsystem: p.subsystem,
        class: p.class,
        revision: p.revision,
        pad: [0; 3],
        bar0_bytes: d.identity.bar0_bytes,
    };
    // SAFETY: `out` is writable (caller contract).
    unsafe { *out = id };
    0
}

/// ★ ABI 7 (2026-09-26): config-space word `idx` the C device must preset read-only
/// (`kf_chip::bar0::config_words` — Hopper+ read PCIe facts by config cycle). Returns 0 and fills
/// `off`/`val`, or -1 past the end / on a bad handle.
///
/// # Safety
/// `off` and `val` are writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kf3_config_word(
    h: *mut c_void,
    idx: u32,
    off: *mut u16,
    val: *mut u32,
) -> i32 {
    let Some(d) = dev(h) else { return -1 };
    let Some(w) = d.config_words.get(idx as usize) else {
        return -1;
    };
    if off.is_null() || val.is_null() {
        return -1;
    }
    // SAFETY: both writable (caller contract).
    unsafe {
        *off = w.off;
        *val = w.value;
    }
    0
}

/// The memory map: writes up to `cap` regions to `out`, returns the total count.
///
/// # Safety
/// `out` is null or writable for `cap` regions.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kf3_memory_map(
    h: *mut c_void,
    bar1: u64,
    bar2: u64,
    out: *mut Kf3Region,
    cap: usize,
) -> i64 {
    let Some(d) = dev(h) else { return -1 };
    let map = d.memory_map(bar1, bar2);
    let regs: Vec<Kf3Region> = map
        .regions
        .iter()
        .map(|r| Kf3Region {
            bar: r.bar.0,
            how: match r.how {
                kf_trap::memmap::Disposition::PlainRam => 0,
                kf_trap::memmap::Disposition::ShadowWriteTrapped => 1,
                kf_trap::memmap::Disposition::HostPassthrough => 2,
                kf_trap::memmap::Disposition::Hole { .. } => 3,
            },
            pad: [0; 6],
            base: r.base,
            len: r.len,
        })
        .collect();
    if !out.is_null() {
        for (i, r) in regs.iter().take(cap).enumerate() {
            // SAFETY: `i < cap`, and `out` is writable for `cap` regions (caller contract).
            unsafe { *out.add(i) = *r };
        }
    }
    regs.len() as i64
}

/// Attach a BAR0 shadow piece (a ROM device's RAM) at BAR0 offset `base`; Rust fills it.
///
/// # Safety
/// `mem` is valid for `len` bytes until the device is unrealized.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kf3_shadow_attach(
    h: *mut c_void,
    base: u64,
    mem: *mut u8,
    len: u64,
) -> i32 {
    let (Some(d), false) = (dev(h), mem.is_null()) else {
        return -1;
    };
    // SAFETY: the caller keeps `[mem, mem+len)` mapped until unrealize.
    d.attach_shadow(base, unsafe { RawRegion::adopt(mem, len as usize) });
    0
}

/// Seal the shadow and publish the GSP registers' initial values.
#[unsafe(no_mangle)]
pub extern "C" fn kf3_shadow_seal(h: *mut c_void) {
    if let Some(d) = dev(h) {
        d.seal_shadow();
    }
}

/// ★ The vCPU path: a BAR0 write.
#[unsafe(no_mangle)]
pub extern "C" fn kf3_bar0_write(h: *mut c_void, off: u64, val: u64, width: u32) {
    if let Some(d) = dev(h) {
        d.bar0_write(off, val, u8::try_from(width).unwrap_or(4));
    }
}

/// ★ w828: a BAR0 READ exit — reached only for a `Disposition::Hole` page (a register whose read
/// has a side effect). Lock-free; never blocks.
#[unsafe(no_mangle)]
pub extern "C" fn kf3_bar0_read(h: *mut c_void, off: u64, width: u32) -> u64 {
    dev(h).map_or(0, |d| d.bar0_read(off, u8::try_from(width).unwrap_or(4)))
}

/// Register guest RAM `[gpa, gpa+len)` at `hva`, backed by the memory backend's `fd` at file
/// offset `fd_off` (`fd < 0`: none).
///
/// # Safety
/// `hva` is valid for `len` bytes, and `fd` (when `>= 0`) stays open, until `kf3_ram_del` for the
/// same `gpa`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kf3_ram_add(
    h: *mut c_void,
    gpa: u64,
    hva: *mut u8,
    len: u64,
    fd: i32,
    fd_off: u64,
) -> i32 {
    let (Some(d), false) = (dev(h), hva.is_null()) else {
        return -1;
    };
    // SAFETY: the caller keeps the RAM mapped until it unregisters it.
    // SAFETY: the caller keeps the RAM mapped, and its backend fd open, until it unregisters it.
    let (mem, fd) = unsafe {
        (
            RawRegion::adopt(hva, len as usize),
            crate::raw_unsafe::BackendFd::adopt(fd),
        )
    };
    d.ram_add(gpa, mem, fd, fd_off);
    0
}

/// ★ P4: the host address backing a plain-RAM (disposition A) region — `[base, base+len)` of
/// `bar` (0: PRAMIN inside BAR0; 1: BAR1; 2: RM's BAR2 = PCI BAR3). Writes `*ptr`; returns 0, or
/// -1 when no window covers the region. The pages live for the process and are never unmapped:
/// every re-point is a `MAP_FIXED` placement inside them.
///
/// # Safety
/// `ptr` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kf3_bar_ram(
    h: *mut c_void,
    bar: u32,
    base: u64,
    len: u64,
    ptr: *mut *mut c_void,
) -> i32 {
    let (Some(d), false) = (dev(h), ptr.is_null()) else {
        return -1;
    };
    let Some(span) = d.window_address(bar, base, len) else {
        return -1;
    };
    // SAFETY: `ptr` is writable (caller contract); the span is handed to QEMU as a memory region's
    // backing for the device's life, which is the use `HostSpan::as_ptr` requires.
    unsafe { *ptr = span.as_ptr().cast::<c_void>() };
    0
}

/// Unregister the guest RAM block at `gpa`.
#[unsafe(no_mangle)]
pub extern "C" fn kf3_ram_del(h: *mut c_void, gpa: u64) {
    if let Some(d) = dev(h) {
        d.ram_del(gpa);
    }
}

/// A one-line status for the QEMU log (written to `buf`, NUL-terminated).
///
/// # Safety
/// `buf` is writable for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kf3_status(h: *mut c_void, buf: *mut c_char, len: usize) {
    let Some(d) = dev(h) else { return };
    let s = d.status_line();
    write_err(buf, len, &s);
}

/// The host usermode window (§53.1 disposition C): `*ptr`, `*len`. Returns 0, or -1 if the session
/// has none. The pages live as long as the device (which lives for the process).
///
/// # Safety
/// `ptr` and `len` are writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kf3_usermode_view(
    h: *mut c_void,
    ptr: *mut *mut c_void,
    len: *mut u64,
) -> i32 {
    let Some(d) = dev(h) else { return -1 };
    let Ok(span) = d.rm.usermode_view() else {
        return -1;
    };
    if ptr.is_null() || len.is_null() {
        return -1;
    }
    // SAFETY: the caller promised both are writable; the span becomes a ROM device's backing for
    // the device's life (the use `HostSpan::as_ptr` requires).
    unsafe {
        *ptr = span.as_ptr().cast::<c_void>();
        *len = span.len() as u64;
    }
    0
}

/// ★ P5 §2.7: the eventfd of MSI-X vector `vector` — the C device wraps it in an EventNotifier and
/// registers it as a KVM irqfd on the vector's MSI route. Returns the fd, or -1.
#[unsafe(no_mangle)]
pub extern "C" fn kf3_irq_fd(h: *mut c_void, vector: u32) -> i32 {
    dev(h).and_then(|d| d.irq_fd(vector as usize)).unwrap_or(-1)
}

/// ★ Whether this device's guest can place BAR1 usermode (doorbell) views — Hopper+ — so the C
/// device must build its overlay pool and register [`kf3_set_bar1_overlay`]. 1 or 0; -1 on a bad
/// handle.
#[unsafe(no_mangle)]
pub extern "C" fn kf3_bar1_follows_guest(h: *mut c_void) -> i32 {
    dev(h).map_or(-1, |d| i32::from(d.plane.doorbell.follows_guest_bar1()))
}

/// ★ Register the C device's BAR1 overlay verb and its pool size (`bar1-overlays`,
/// `V3_BAR1_DOORBELL.md` §3.1). Returns 0, or -1 (bad handle, null verb, zero slots, or already
/// registered).
///
/// # Safety
/// `f` must be callable from any non-vCPU thread with `opaque` for the process's lifetime and must
/// not block (it queues; the main loop applies and answers through [`kf3_bar1_overlay_done`]).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kf3_set_bar1_overlay(
    h: *mut c_void,
    f: Option<crate::raw_unsafe::OverlayFn>,
    opaque: *mut c_void,
    slots: u32,
) -> i32 {
    let (Some(d), Some(f)) = (dev(h), f) else {
        return -1;
    };
    if slots == 0 {
        return -1;
    }
    // SAFETY: forwarded from this function's contract.
    let hook = unsafe { crate::raw_unsafe::OverlayHook::adopt(f, opaque) };
    if d.bar1_overlay.set(hook, slots as usize) {
        0
    } else {
        -1
    }
}

/// ★ The main loop applied BAR1 overlay change `seq` with result `rc` (0, or a negative errno).
/// Main-loop thread, BQL held; posts and wakes the VA thread — never blocks.
#[unsafe(no_mangle)]
pub extern "C" fn kf3_bar1_overlay_done(h: *mut c_void, seq: u64, rc: i32) {
    if let Some(d) = dev(h) {
        d.bar1_overlay_done(seq, rc);
    }
}

/// ★ THE vCPU PATH for a write into a BAR1 usermode overlay: `vf_rel` = offset inside the 64 KiB
/// usermode page (the overlay aliases it, so QEMU hands us that offset directly). Lock-free.
#[unsafe(no_mangle)]
pub extern "C" fn kf3_bar1_usermode_write(h: *mut c_void, vf_rel: u64, val: u64, width: u32) {
    if let Some(d) = dev(h) {
        d.bar1_usermode_write(vf_rel, val, u8::try_from(width).unwrap_or(4));
    }
}

/// ★ ABI 10 (`v3-display2`'s 9): one frame of the virtual display for QEMU's console
/// (`display=on`). ★ ABI 14: `len`. ⊘ No `Debug`: `data` is an address.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Kf3Frame {
    /// First pixel: page-locked host memory the display plane leaked for the process.
    pub data: *mut u8,
    /// ★ ABI 14: bytes readable at `data` — the frame's own length. `[data, data+len)` stays valid
    /// for the life of the process.
    pub len: u64,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Bytes per row.
    pub stride: u32,
    /// `kf_disp::scanout::PixelFormat` code (1 xrgb8888, 2 xbgr8888, 3 rgb565, 4 x2rgb10, 5 x2bgr10).
    pub format: u32,
    /// Increases with every new frame.
    pub serial: u64,
}

/// Bytes per pixel of a console format code, or `None` for a code the console has no format for.
#[must_use]
pub fn frame_bpp(format: u32) -> Option<u32> {
    match format {
        1 | 2 | 4 | 5 => Some(4),
        3 => Some(2),
        _ => None,
    }
}

/// ★★★ **V13, pure — a frame of `len` bytes may be read as `width × height` at `stride`** in
/// `format`: a known format; `width, height ≥ 1`; `width`, `height` and `stride` each fit a C `int`
/// (QEMU's surface takes them as `int`); `stride` a multiple of 4; a row (`width × bpp` bytes, in
/// `u64`) no wider than `stride`; and the WHOLE `stride × height` inside `len` — QEMU's D-Bus
/// listener sends exactly `stride × height` bytes (`ui/dbus-listener.c:720-722`). `kf3.h`'s
/// `kf3_frame_ok` is the same predicate (`tests/wire_mirror.rs` runs both over one grid).
///
/// # Errors
/// The failed bound, by name.
pub fn check_frame_geometry(
    len: u64,
    width: u32,
    height: u32,
    stride: u32,
    format: u32,
) -> Result<u32, &'static str> {
    let bpp = frame_bpp(format).ok_or("an unknown pixel format")?;
    if width == 0 || height == 0 {
        return Err("an empty frame");
    }
    let int = i32::MAX as u32;
    if width > int || height > int || stride > int {
        return Err("a dimension that does not fit a C int");
    }
    if !stride.is_multiple_of(4) {
        return Err("a stride that is not a multiple of 4");
    }
    if u64::from(width) * u64::from(bpp) > u64::from(stride) {
        return Err("a row wider than the stride");
    }
    if u64::from(stride) * u64::from(height) > len {
        return Err("stride x height leaves the frame");
    }
    Ok(bpp)
}

/// ★★ **V13 — what the console may be handed**: `Err(-1)` when the slot names no frame (nothing
/// new to show), `Err(-2)` when the geometry does not fit THAT frame's own length (a torn read of
/// the id and the geometry, or a worker bug — refused, never read past the frame), else the span.
///
/// # Errors
/// `-1` or `-2`, as above.
pub fn check_frame(v: &crate::display::FrameView) -> Result<kf_linux_raw::StaticSpan, i32> {
    let span = v.span.ok_or(-1)?;
    check_frame_geometry(span.len() as u64, v.width, v.height, v.stride, v.format)
        .map_err(|_| -2)?;
    Ok(span)
}

/// ★ ABI 10 (`v3-display2`'s 9; QEMU's main thread, the console's `gfx_update`): the newest frame
/// of the virtual display. `0` and `*out` filled — `[data, data+len)` is valid for the process and
/// not written until the next call (the worker never fills the frame the console shows); `-1` when
/// there is no new frame (before the first, without a display, or a slot naming no frame: keep the
/// current surface); ★ ABI 14: `-2` when the frame was refused by [`check_frame`] (`*out` untouched;
/// the device shows QEMU's placeholder).
///
/// # Safety
/// `out` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kf3_display_frame(h: *mut c_void, out: *mut Kf3Frame) -> i32 {
    let (Some(d), false) = (dev(h), out.is_null()) else {
        return -1;
    };
    let Some(dp) = d.display else {
        return -1;
    };
    let Some(f) = dp.console.take() else {
        return -1;
    };
    let span = match check_frame(&f) {
        Ok(span) => span,
        Err(rc) => {
            if rc == -2
                && dp
                    .counters
                    .console_refused
                    .fetch_add(1, core::sync::atomic::Ordering::Relaxed)
                    == 0
            {
                eprintln!(
                    "kf3: display: a console frame was REFUSED ({}x{} stride {} format {} does not fit its {}-byte frame); the console shows a placeholder",
                    f.width,
                    f.height,
                    f.stride,
                    f.format,
                    f.span.map_or(0, |s| s.len())
                );
            }
            return rc;
        }
    };
    // SAFETY: `out` is writable (caller contract). The span is a `StaticSpan`: an owned,
    // private-anonymous mapping the display plane leaked, so it is never unmapped; `check_frame`
    // refused any frame without `stride × height ≤ len`, and C reads only `[data, data+len)`
    // (`kf3.h`), as `HostSpan::as_ptr`'s contract allows a VMM console surface.
    unsafe {
        *out = Kf3Frame {
            data: span.host_span().as_ptr(),
            len: span.len() as u64,
            width: f.width,
            height: f.height,
            stride: f.stride,
            format: f.format,
            serial: f.serial,
        };
    }
    0
}

/// ★ ABI 10 (`v3-ioeventfd`'s 9): the doorbell register's offset inside the 64 KiB usermode page
/// (the same register through BAR0's usermode piece and through every Hopper+ BAR1 view of it) —
/// what the C device's memory listener looks for to report doorbell sites. -1 on a bad handle.
#[unsafe(no_mangle)]
pub extern "C" fn kf3_doorbell_page_offset(h: *mut c_void) -> i64 {
    dev(h).map_or(-1, |d| {
        i64::try_from(d.plane.doorbell.offset()).unwrap_or(-1)
    })
}

/// ★ ABI 10 (`v3-ioeventfd`'s 9, `docs/design/V3_DOORBELL_IOEVENTFD.md`): turn the doorbell FAST
/// PATH on — hand the device the C device's `KVM_IOEVENTFD` verb and the placement budget
/// (`doorbell-ioeventfd-max`). Called only when the `doorbell-ioeventfd` property is on; without it
/// every doorbell stays trapped. Returns 0, or -1 (bad handle, null verb, already on).
///
/// # Safety
/// `f` must be callable from any non-vCPU thread with `opaque` for the process's lifetime, must only
/// issue `KVM_IOEVENTFD` with the arguments given, and must not retain the fd.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kf3_set_ioeventfd(
    h: *mut c_void,
    f: Option<crate::raw_unsafe::IoeventfdFn>,
    opaque: *mut c_void,
    budget: u32,
) -> i32 {
    let (Some(d), Some(f)) = (dev(h), f) else {
        return -1;
    };
    // SAFETY: forwarded from this function's contract.
    let hook = unsafe { crate::raw_unsafe::IoeventfdHook::adopt(f, opaque) };
    if d.enable_doorbell_fast_path(hook, budget as usize) {
        0
    } else {
        -1
    }
}

/// ★ ABI 10 (`v3-ioeventfd`'s 9): a doorbell register became visible (`add` = 1) at
/// guest-physical `gpa`, or went away (`add` = 0) — the C device's memory listener (main loop, BQL
/// held), for BAR0's usermode piece and every Hopper+ BAR1 view. Never on a vCPU's MMIO path.
#[unsafe(no_mangle)]
pub extern "C" fn kf3_doorbell_site(h: *mut c_void, gpa: u64, add: u32) {
    if let Some(d) = dev(h) {
        if add != 0 {
            d.dbfast.site_add(gpa);
        } else {
            d.dbfast.site_del(gpa);
        }
    }
}

/// ★ ABI 11 (`docs/design/V3_DISPLAY.md` §4.11.6): the boot display's option ROM — the embedded GOP
/// driver wrapped with this device's ids and its `KFGP` descriptor, packed at realize (`gop=on`).
/// `0` with `*rom`/`*rom_len` filled: the bytes live for the process (the device is never freed), and
/// the C device copies them into its ROM BAR. `-1` with `gop=off`, or on a bad handle.
///
/// # Safety
/// `rom` and `rom_len` are writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kf3_option_rom(
    h: *mut c_void,
    rom: *mut *const u8,
    rom_len: *mut u64,
) -> i32 {
    let (Some(d), false, false) = (dev(h), rom.is_null(), rom_len.is_null()) else {
        return -1;
    };
    let Some(bytes) = d.option_rom() else {
        return -1;
    };
    // SAFETY: both writable (caller contract); the bytes are the leaked device's for the process.
    unsafe {
        *rom = bytes.as_ptr();
        *rom_len = bytes.len() as u64;
    }
    0
}

/// Stop the device's threads (the device itself lives for the process).
#[unsafe(no_mangle)]
pub extern "C" fn kf3_unrealize(h: *mut c_void) {
    if let Some(d) = dev(h) {
        d.stop();
    }
}
