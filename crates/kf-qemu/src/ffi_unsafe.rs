//! ★ The `extern "C"` surface the kf3-gpu QOM device calls. Every entry point validates its handle
//! and pointers and hands off to the safe [`crate::device::Device`] immediately.

use crate::device::{Config, Device};
use crate::raw_unsafe::RawRegion;
use core::ffi::{c_char, c_void};
use std::ffi::CStr;
use std::os::unix::ffi::OsStrExt as _;

/// Wire ABI of this surface; the C device refuses a mismatched archive.
/// ★ 10 (2026-09-30, `v3-mc22`): the union of two INDEPENDENT 9s — `v3-display2`'s frame hand-off
/// ([`Kf3Frame`], [`kf3_display_frame`]) and `v3-ioeventfd`'s doorbell fast path
/// ([`kf3_doorbell_page_offset`], [`kf3_set_ioeventfd`], [`kf3_doorbell_site`]). The two 9s name
/// different surfaces, so an archive from either branch must fail the device's check: one new
/// number above both. `tests/wire_mirror.rs` compiles every entry point here against `kf3.h`.
/// ★ 11 (2026-10-03, `v3-gop-kf3`, `docs/design/V3_DISPLAY.md` §4.11): the boot display —
/// [`kf3_realize`] takes `gop`, and [`kf3_option_rom`] hands the C device the ROM to register.
/// ★ 12 (2026-10-03, `v3-broker`, display step 3 — `docs/design/V3_DISPLAY.md` §8): ONE number above
/// master's 11 for the whole broker surface. ⊘ The branch had numbered its own steps 11, 12 and 13
/// before master's boot display took 11; the merge folds them into this one bump:
/// `kf3_realize` gains `display_broker` (after `gop`), whose word carries the broker in bit 0 and
/// `display-broker-vram` in bits 1-2 (0 auto, 1 on, 2 off; [`kf_broker::gpucopy::VramMode::from_abi`]);
/// the broker relay's surface ([`Kf3BrokerEvent`], [`kf3_broker_start`], [`kf3_broker_frame_fd`],
/// [`kf3_broker_ready`], [`kf3_broker_stop`]); [`kf3_display_ui_info`] (the console's `ui_info`
/// hook) and the broker's `SURFACE` event (kind 8). (⊘ "`v3-dispsw-exp` takes 13 when it merges"
/// is corrected by the registry under 16: 13 is that branch's own number.)
/// ★ Still 12 on 2026-10-04 (§8.13): the console's cursor in hover ([`Kf3Cursor`],
/// [`kf3_display_cursor`], [`kf3_display_cursor_pixels`], and since the review of the same day
/// [`kf3_display_cursor_done`]) joins the broker's surface while it is unmerged — the bump is per
/// surface reaching master, and an archive without these symbols fails to LINK with a kf3.c that
/// calls them, never at run time.
/// ★ 16 (2026-10-04, `v3-maxfps`, `docs/design/V3_DISPLAY.md` §8.16, `OWNER_RULINGS.md` §M): the
/// configurable frame-rate bound — [`kf3_realize`] gains `display_max_fps` after `display_broker`
/// (whole Hz, 0 unset), and the console's on-demand refresh joins the surface
/// ([`kf3_display_refresh`], [`kf3_display_refresh_fd`], [`kf3_display_refresh_drain`]). The
/// registry (one number per shape that reached a binary, never reused): 11 master (GOP), 12
/// `v3-broker` (and `v3-windows`, renumbered at its merge), 13 `v3-dispsw-exp`, 14 reserved
/// (broker-on-13, `v3-cand-1`), 15 `v3-viommu`, 16 this branch — cut from `v3-broker` `82f98f42`,
/// so a merge with 13, 14 or 15 takes a new number.
pub const KF3_ABI: u32 = 16;

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
    display_broker: u32,
    display_max_fps: u32,
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
    let vram = match kf_broker::gpucopy::VramMode::from_abi(display_broker) {
        Ok(v) => v,
        Err(e) => {
            write_err(err, err_len, &e);
            return -1;
        }
    };
    let cfg = Config {
        gpu_minor,
        fb_mb,
        bar1_bytes,
        bar2_bytes,
        guest_driver: guest,
        display: display != 0,
        gop: gop != 0,
        display_broker: vram.is_some(),
        display_broker_vram: vram.unwrap_or_default(),
        display_max_fps,
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
/// (`display=on`).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Kf3Frame {
    /// First pixel: page-locked host memory the display worker owns for the process.
    pub data: *mut u8,
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

/// ★ ABI 12 (display step 3): one input event from the display broker for the C device to inject
/// (`kind`: 1 key, 2 button, 3 absolute, 4 relative, 5 wheel, 6 grab, 7 close; 8 surface —
/// `x`, `y` = the broker window's size, `w0` = its refresh in mHz). Every value is
/// already bounded by the relay (`kf_broker::Input`); the C device still checks a key code
/// against QEMU's own map.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Kf3BrokerEvent {
    /// `KF3_BROKER_*` (kf3.h).
    pub kind: u32,
    /// Key/button code, x, dx, wheel direction (+1 up, -1 down), grab on, close forced.
    pub x: i32,
    /// Pressed, y, dy.
    pub y: i32,
    /// The absolute range's width.
    pub w0: u32,
    /// The absolute range's height.
    pub w1: u32,
}

/// ★ §8.13 (KF3 ABI 12's broker surface, before it reaches master): what QEMU's console is told
/// about the guest's cursor while a cursor-capable broker hovers ([`kf3_display_cursor`]). `what`:
/// bit 0 DEFINE — `width` x `height` with the hot spot (`width` = 0: the hidden cursor), its pixels
/// from [`kf3_display_cursor_pixels`]; bit 1 MOUSE — `dpy_mouse_set(x, y, on)`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Kf3Cursor {
    /// `KF3_CURSOR_DEFINE` | `KF3_CURSOR_MOUSE`.
    pub what: u32,
    /// The image's width (0: the hidden cursor) …
    pub width: u32,
    /// … and height (at most 256 each).
    pub height: u32,
    /// The hot spot's column …
    pub hot_x: u32,
    /// … and row, inside the image.
    pub hot_y: u32,
    /// The guest pointer on the console's frame: column …
    pub x: i32,
    /// … and row.
    pub y: i32,
    /// Whether the cursor is shown.
    pub on: u32,
}

impl Kf3BrokerEvent {
    fn of(i: kf_broker::Input) -> Kf3BrokerEvent {
        use kf_broker::Input as I;
        let (kind, x, y, w0, w1) = match i {
            I::Key { code, down } => (1, i32::from(code), i32::from(down), 0, 0),
            I::Btn { code, down } => (2, i32::from(code), i32::from(down), 0, 0),
            I::Abs { x, y, w, h } => (
                3,
                x,
                y,
                u32::try_from(w).unwrap_or(1),
                u32::try_from(h).unwrap_or(1),
            ),
            I::Rel { dx, dy } => (4, dx, dy, 0, 0),
            I::Wheel { up } => (5, if up { 1 } else { -1 }, 0, 0, 0),
            I::Grab(on) => (6, i32::from(on), 0, 0, 0),
            I::Close { force } => (7, i32::from(force), 0, 0, 0),
            I::Surface { w, h, mhz } => (8, w, h, mhz, 0),
        };
        Kf3BrokerEvent { kind, x, y, w0, w1 }
    }
}

/// ★ ABI 10 (`v3-display2`'s 9; QEMU's main thread, the console's `gfx_update`): the newest frame
/// of the virtual display. `0` and `*out` filled — the memory stays valid, and is not written, until
/// the next call (the worker never fills the frame the console shows); `-1` before the first frame,
/// or without a display.
///
/// # Safety
/// `out` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kf3_display_frame(h: *mut c_void, out: *mut Kf3Frame) -> i32 {
    // §R: validated at the boundary — null AND alignment (the review of 2026-10-04)
    let (Some(d), false) = (dev(h), out.is_null() || !out.is_aligned()) else {
        return -1;
    };
    let Some(f) = d.display.and_then(|dp| dp.console.take()) else {
        return -1;
    };
    let fr = Kf3Frame {
        data: f.addr as *mut u8,
        width: f.width,
        height: f.height,
        stride: f.stride,
        format: f.format,
        serial: f.serial,
    };
    // SAFETY: `out` is writable (caller contract).
    unsafe { *out = fr };
    0
}

/// ★ ABI 16 (`display-max-fps` D2.3, `docs/design/V3_DISPLAY.md` §8.16; main thread, the console's
/// `gfx_update` — a `screendump` among its callers): ask for a frame no older than now. `1`: the
/// worker will signal [`kf3_display_refresh_fd`] when the newest frame is (it checks the frame at the
/// console head's next tick and sends it if it changed; at once when nothing can be copied); `0`:
/// no answer will come (no display, or no descriptor) — the caller answers its waiter itself.
/// Lock-free: one atomic and one eventfd write.
#[unsafe(no_mangle)]
pub extern "C" fn kf3_display_refresh(h: *mut c_void) -> i32 {
    match dev(h).and_then(|d| d.display) {
        Some(dp) if dp.request_refresh() => 1,
        _ => 0,
    }
}

/// ★ ABI 16: the descriptor that becomes readable when refresh requests were served (the C device
/// watches it on its main loop), or -1 (no display, none could be made).
#[unsafe(no_mangle)]
pub extern "C" fn kf3_display_refresh_fd(h: *mut c_void) -> i32 {
    dev(h)
        .and_then(|d| d.display)
        .map_or(-1, |dp| dp.console.refresh_fd())
}

/// ★ ABI 16 (main loop, the refresh descriptor's handler): consume its readiness.
#[unsafe(no_mangle)]
pub extern "C" fn kf3_display_refresh_drain(h: *mut c_void) {
    if let Some(dp) = dev(h).and_then(|d| d.display) {
        dp.console.refresh_drain();
    }
}

/// ★ §8.13 (ABI 12, main thread: the console's `gfx_update`, AFTER it took its frame, and each
/// broker pump): what QEMU's console should be told about the guest's cursor now — the
/// coordinator's decision of 2026-10-04: while a cursor-capable broker hovers the frames carry no
/// cursor (§O), so the console gets it through QEMU's cursor API (VNC shows it as a real pointer);
/// under grab, or with no such broker, it stays composed. The define follows the frame the console
/// shows and is paced (`kf_broker::ConsoleCursor::poll`). Returns `out.what` (0: nothing to do,
/// also without a display or a broker; `*out` untouched then); after a nonzero return the caller
/// reports what it applied with [`kf3_display_cursor_done`].
///
/// # Safety
/// `out` is writable (null and misalignment are refused here).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kf3_display_cursor(h: *mut c_void, out: *mut Kf3Cursor) -> i32 {
    let (Some(dp), false) = (
        dev(h).and_then(|d| d.display),
        out.is_null() || !out.is_aligned(),
    ) else {
        return 0;
    };
    let Some(seat) = dp.broker.as_ref() else {
        return 0;
    };
    let u = seat.console_cursor(
        dp.console.cursor_point(),
        dp.console.shown_frame(),
        dp.console.now_ms(),
    );
    let mut c = Kf3Cursor::default();
    if let Some(define) = u.define {
        c.what |= 1;
        if let Some(sh) = define {
            (c.width, c.height, c.hot_x, c.hot_y) = (sh.width, sh.height, sh.hot.0, sh.hot.1);
        }
    }
    if let Some((x, y, on)) = u.mouse {
        c.what |= 2;
        (c.x, c.y, c.on) = (x, y, u32::from(on));
    }
    if c.what == 0 {
        return 0;
    }
    // SAFETY: `out` is writable (caller contract), checked non-null and aligned above.
    unsafe { *out = c };
    i32::try_from(c.what).unwrap_or(0)
}

/// ★ §8.13 (ABI 12, main thread, right after acting on a nonzero [`kf3_display_cursor`]): what the C
/// device APPLIED of it — `KF3_CURSOR_DEFINE` when `dpy_cursor_define` ran, `KF3_CURSOR_MOUSE`
/// when `dpy_mouse_set` ran (only under an absolute pointer). A part not applied is handed out
/// again at a later poll (the review of 2026-10-04: Rust used to believe the console held a
/// cursor the C side had refused to define, and never retried).
#[unsafe(no_mangle)]
pub extern "C" fn kf3_display_cursor_done(h: *mut c_void, applied: u32) {
    if let Some(seat) = dev(h)
        .and_then(|d| d.display)
        .and_then(|dp| dp.broker.as_ref())
    {
        seat.console_cursor_done(applied & 1 != 0, applied & 2 != 0);
    }
}

/// The most words [`kf3_display_cursor_pixels`] writes: a 256x256 cursor (the broker's bound,
/// `kf_broker::wire::CURSOR_MAX_DIM`).
const CURSOR_MAX_WORDS: u32 = 256 * 256;

/// ★ §8.13 (ABI 12, main thread, right after a DEFINE from [`kf3_display_cursor`]): the defined
/// image's pixels into `data` — QEMU's `QEMUCursor` data, one host-endian `0xAARRGGBB` word per
/// pixel, PREMULTIPLIED (what VNC's alpha cursor carries; ⊘ straight until the review of
/// 2026-10-04) — copied from kayfabe's own copy of the image, never guest memory.
/// `words` must be exactly the defined `width * height`. 0, or -1 with nothing written.
///
/// # Safety
/// `data` is null or writable for `words` aligned `u32`s (validated here: null, misalignment and
/// more than 256x256 words are refused before a byte is written).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kf3_display_cursor_pixels(
    h: *mut c_void,
    data: *mut u32,
    words: u32,
) -> i32 {
    if data.is_null() || !data.is_aligned() || words == 0 || words > CURSOR_MAX_WORDS {
        return -1;
    }
    let Some(seat) = dev(h)
        .and_then(|d| d.display)
        .and_then(|dp| dp.broker.as_ref())
    else {
        return -1;
    };
    // SAFETY: `data` is non-null, aligned, and writable for `words` u32s (caller contract);
    // `words` is at most 256x256, so the slice is at most 256 KiB, and it lives only for the call.
    let out = unsafe { core::slice::from_raw_parts_mut(data, words as usize) };
    match seat.console_cursor_pixels(out) {
        Ok(()) => 0,
        Err(_) => -1,
    }
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

/// ★ ABI 12 (display step 3, `docs/design/V3_DISPLAY.md` §8): start the display-broker relay —
/// QEMU's main loop, BQL held, after `kf3_realize` with `display_broker` = 1. `path` is the
/// broker's socket (absolute, shorter than `sun_path`); `extra_uid` is the `display-broker-uid`
/// property — `-1` none, or one more uid accepted as the broker (any other value is refused by
/// name); `watch`/`timer` are the C device's fd-handler and timer verbs, called back only from
/// inside the `kf3_broker_*` entries. Returns 0, or -1 with a message (a broker that is not
/// running yet is NOT an error: the first attempt runs from the main loop's timer, and a failed
/// one is retried in the background).
///
/// # Safety
/// `path` is a NUL-terminated string; `err` is null or writable for `err_len` bytes; `watch` and
/// `timer` must be callable with `opaque` on the main loop for the device's lifetime, must not
/// re-enter a `kf3_broker_*` entry, and must not block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kf3_broker_start(
    h: *mut c_void,
    path: *const c_char,
    extra_uid: i64,
    watch: Option<crate::raw_unsafe::BrokerWatchFn>,
    timer: Option<crate::raw_unsafe::BrokerTimerFn>,
    opaque: *mut c_void,
    now_ms: u64,
    err: *mut c_char,
    err_len: usize,
) -> i32 {
    let (Some(d), false, Some(watch), Some(timer)) = (dev(h), path.is_null(), watch, timer) else {
        write_err(err, err_len, "kf3_broker_start: a null argument");
        return -1;
    };
    let Some(seat) = d.display.and_then(|dp| dp.broker.as_ref()) else {
        write_err(
            err,
            err_len,
            "display-broker needs display=on and a device realized with the broker's frames",
        );
        return -1;
    };
    // SAFETY: the caller promises a NUL-terminated string.
    let p = unsafe { CStr::from_ptr(path) };
    let path = std::path::PathBuf::from(std::ffi::OsStr::from_bytes(p.to_bytes()));
    // SAFETY: forwarded from this function's contract.
    let hooks = unsafe { crate::raw_unsafe::BrokerHooks::adopt(watch, timer, opaque) };
    match seat.start(&path, extra_uid, hooks, now_ms) {
        Ok(()) => 0,
        Err(e) => {
            write_err(err, err_len, &e);
            -1
        }
    }
}

/// ★ ABI 12: the display worker's frame eventfd, for the C device to watch for readability
/// (main loop); -1 without a broker.
#[unsafe(no_mangle)]
pub extern "C" fn kf3_broker_frame_fd(h: *mut c_void) -> i32 {
    dev(h)
        .and_then(|d| d.display)
        .and_then(|dp| dp.broker.as_ref())
        .map_or(-1, crate::broker::BrokerSeat::frame_fd)
}

/// ★ ABI 12 (main loop, BQL held): something the relay waits on is ready — `fd` is the socket
/// (`rd`/`wr` say which), the frame eventfd, or -1 for the relay's timer. Writes at most `cap`
/// input events to `out` and returns how many (never negative; 0 on a bad handle). Reads at most
/// `cap` packets from the socket; level-triggered readiness delivers the rest.
///
/// # Safety
/// `out` is null or writable for `cap` events.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kf3_broker_ready(
    h: *mut c_void,
    fd: i32,
    rd: u32,
    wr: u32,
    now_ms: u64,
    out: *mut Kf3BrokerEvent,
    cap: u32,
) -> i32 {
    let Some(dp) = dev(h).and_then(|d| d.display) else {
        return 0;
    };
    let Some(seat) = dp.broker.as_ref() else {
        return 0;
    };
    let cap = if out.is_null() { 0 } else { cap as usize };
    let mut evs = Vec::with_capacity(cap.min(kf_broker::conn::READ_BATCH));
    let active = seat.ready(fd, rd != 0, wr != 0, now_ms, &mut evs, cap.max(1));
    if active {
        // broker activity keeps the refresh clock at the watched rate; it asks for a host copy
        // only while the broker is fed through host memory (§8.11, two demand signals)
        dp.console.note_broker_demand();
    }
    // ★ §8.16: a session that just became active is a new viewer — the next check sends
    if seat.became_active(active) {
        dp.console.note_new_watcher();
    }
    let n = evs.len().min(cap);
    for (i, e) in evs.iter().take(n).enumerate() {
        // SAFETY: `i < n <= cap`, and `out` is writable for `cap` events (caller contract).
        unsafe { *out.add(i) = Kf3BrokerEvent::of(*e) };
    }
    i32::try_from(n).unwrap_or(0)
}

/// ★ ABI 12 (main loop, device exit, BEFORE the console closes): stop the relay — unwatch, close,
/// no timer.
#[unsafe(no_mangle)]
pub extern "C" fn kf3_broker_stop(h: *mut c_void) {
    if let Some(seat) = dev(h)
        .and_then(|d| d.display)
        .and_then(|dp| dp.broker.as_ref())
    {
        seat.stop();
    }
}

/// ★ ABI 12 (display step 3c; QEMU's main loop, the console's `ui_info` hook after QEMU's 1 s
/// coalescing): the UI wants head `head` to be `width` x `height` at `refresh_mhz` (0: unknown). The
/// worker authors a new monitor (EDID) and, when a hotplug registration is live, the drainer posts
/// the hotplug. Lock-free. Returns 0, or -1 (no display, a head without a console, a zero size).
#[unsafe(no_mangle)]
pub extern "C" fn kf3_display_ui_info(
    h: *mut c_void,
    head: u32,
    width: u32,
    height: u32,
    refresh_mhz: u32,
) -> i32 {
    match dev(h).and_then(|d| d.display) {
        Some(dp) if dp.request_ui(head, width, height, refresh_mhz) => 0,
        _ => -1,
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
