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
/// the broker relay's surface (`Kf3BrokerEvent`, [`kf3_broker_start`], [`kf3_broker_frame_fd`],
/// [`kf3_broker_ready`], [`kf3_broker_stop`]); [`kf3_display_ui_info`] (the console's `ui_info`
/// hook) and the broker's `SURFACE` event (kind 8). (⊘ "`v3-dispsw-exp` takes 13 when it merges"
/// is corrected by the registry under 16: 13 is that branch's own number.)
/// ★ Still 12 on 2026-10-04 (§8.13): the console's cursor in hover (`Kf3Cursor`,
/// `kf3_display_cursor`, `kf3_display_cursor_pixels`, and since the review of the same day
/// `kf3_display_cursor_done`) joins the broker's surface while it is unmerged — the bump is per
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
/// ★ 18 (2026-10-04, candidate 2): merge ABI 13's x11_dispsw with ABI 16's broker,
/// cursor and capped-refresh surface. The realize tail is gop, x11_dispsw, display_broker,
/// display_max_fps. ABI 17 was already built by the earlier display scratch integration
/// (with another argument order); neither 14, 16 nor 17 can name this interface.
/// 19 (2026-10-05, Windows/P1/P2 integration): append the signed-GOP path
/// after display_max_fps, retaining every ABI-18 display/broker entry point.
/// 20 (2026-10-05): the optional read-only host timer mapping adds HostTimer and
/// [`kf3_timer_view`] to ABI 19; the old Windows branch called its narrower surface 13.
/// 21 (2026-10-07): ABI 20 plus the default-off `KF3_BAR0_TRACE` diagnostic's read-trap verb
/// ([`kf3_set_read_trap`], `crate::raw_unsafe::ReadTrapFn`).
/// 22 (2026-10-08, `claude/gpu-uuid-per-vm-20261008`): ABI 21 plus the per-VM GPU UUID —
/// [`kf3_realize`] gains `gpu_uuid`, `vm_id` (both nullable strings) and `pci_devfn` after
/// `gop_efi`.
/// 24 (2026-10-09, `claude/kf3-read-trace-20261008`): ABI 22 plus the default-off BAR0 trace
/// mode's verbs ([`kf3_trace_mode`], [`kf3_trace_piece`], [`kf3_trace_admit`], [`kf3_trace_name`],
/// [`kf3_trace_report`]; `crate::readtrace`). 23 is `0da871c1`'s thin input/cursor shim (another
/// branch), so this surface takes the next number.
/// ★ 23 (2026-10-08, `claude/input-sink-trait-20261008`, `OWNER_RULINGS.md` §V,
/// `docs/design/V3_DISPLAY.md` §8.20): ABI 22 with the broker's input and the console cursor
/// behind kf-broker's VMM-neutral `InputSink`/`CursorSink` — [`kf3_broker_start`] gains `ops`
/// ([`Kf3InputOps`], before `opaque`); [`kf3_broker_ready`] loses its event array (it delivers
/// through the verbs); [`kf3_display_cursor_apply`] replaces `kf3_display_cursor`,
/// `kf3_display_cursor_pixels` and `kf3_display_cursor_done`; `Kf3BrokerEvent` and `Kf3Cursor`
/// give way to [`Kf3Pointer`] and [`Kf3InputOps`]. A branch that took 23 meanwhile takes a new
/// number at its merge.
/// 25 (2026-10-09, the merge of both at `claude/windows-reset-20261009`): 24's trace verbs AND
/// 23's input/cursor verbs (disjoint surfaces).
pub const KF3_ABI: u32 = 25;

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
/// `guest_driver`, `gop_efi`, `gpu_uuid` and `vm_id` are each null or a NUL-terminated string; `out` is writable; `err`
/// is null or writable for `err_len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kf3_realize(
    gpu_minor: u32,
    fb_mb: u64,
    bar1_bytes: u64,
    bar2_bytes: u64,
    guest_driver: *const c_char,
    display: u32,
    gop: u32,
    x11_dispsw: u32,
    display_broker: u32,
    display_max_fps: u32,
    gop_efi: *const c_char,
    gpu_uuid: *const c_char,
    vm_id: *const c_char,
    pci_devfn: u32,
    out: *mut *mut c_void,
    err: *mut c_char,
    err_len: usize,
) -> i32 {
    // ★ ABI 12: both string arguments through ONE conversion (and one audited block): null or empty
    // is unset.
    let text = |p: *const c_char| -> Option<String> {
        if p.is_null() {
            return None;
        }
        // SAFETY: `p` is `guest_driver`, `gop_efi`, `gpu_uuid` or `vm_id`, each of which the caller promises is null
        // (returned above) or a NUL-terminated string.
        Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()).filter(|s| !s.is_empty())
    };
    let vram = match kf_broker::gpucopy::VramMode::from_abi(display_broker) {
        Ok(v) => v,
        Err(e) => {
            write_err(err, err_len, &e);
            return -1;
        }
    };
    let guest = text(guest_driver);
    let gop_efi = text(gop_efi);
    let gpu_uuid = text(gpu_uuid);
    let vm_id = text(vm_id);
    let cfg = Config {
        gpu_minor,
        fb_mb,
        bar1_bytes,
        bar2_bytes,
        guest_driver: guest,
        display: display != 0,
        gop: gop != 0,
        x11_dispsw: x11_dispsw != 0,
        display_broker: vram.is_some(),
        display_broker_vram: vram.unwrap_or_default(),
        display_max_fps,
        gop_efi,
        gpu_uuid,
        vm_id,
        pci_devfn,
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
                kf_trap::memmap::Disposition::HostTimer => 4,
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
/// has a side effect), and, while the default-off `KF3_BAR0_TRACE` diagnostic holds a window open,
/// for the shadow pieces (answered from the same shadow). Lock-free; never blocks.
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

/// The optional native timer backing, which may only be installed read-only in the guest.
/// # Safety
/// `ptr` and `len` must be aligned and writable; the returned mapping must not outlive the device.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kf3_timer_view(
    h: *mut c_void,
    ptr: *mut *mut c_void,
    len: *mut u64,
) -> i32 {
    let Some(d) = dev(h) else { return -1 };
    let Some(timer) = &d.timer else { return -1 };
    if ptr.is_null() || len.is_null() || !ptr.is_aligned() || !len.is_aligned() {
        return -1;
    }
    let span = timer.view();
    // SAFETY: caller's writable outputs; backing is valid for the device's lifetime.
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

/// ★ DIAGNOSTIC (`KF3_BAR0_TRACE`, `crate::bar0trace`): register the C device's BAR0 read-trap
/// verb. Returns 0 (registered; it is called only while the flag is on), or -1 (bad handle, null
/// verb, or already registered).
///
/// # Safety
/// `f` must be callable from any non-vCPU thread with `opaque` for the process's lifetime and must
/// not block (it schedules a main-loop bottom half).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kf3_set_read_trap(
    h: *mut c_void,
    f: Option<crate::raw_unsafe::ReadTrapFn>,
    opaque: *mut c_void,
) -> i32 {
    let (Some(d), Some(f)) = (dev(h), f) else {
        return -1;
    };
    // SAFETY: forwarded from this function's contract.
    let hook = unsafe { crate::raw_unsafe::ReadTrapHook::adopt(f, opaque) };
    if d.chans.bar0trace.set_hook(hook) {
        0
    } else {
        -1
    }
}

/// ★ ABI 24, DIAGNOSTIC (`KF3_BAR0_READ_TRACE`, default off; [`crate::readtrace`]): 1 when the
/// BAR0 trace mode is on, 0 when it is off (the C device then takes none of its paths: ROMD on,
/// irqfd MSI, no trace call), -1 on a bad handle.
#[unsafe(no_mangle)]
pub extern "C" fn kf3_trace_mode(h: *mut c_void) -> i32 {
    dev(h).map_or(-1, |d| i32::from(d.trace.on()))
}

/// ★ ABI 24 (trace mode only): 1 when the shadow piece `[base, base + len)` must serve its reads
/// by exit (it overlaps a selected read range), else 0. Always 0 with the mode off.
#[unsafe(no_mangle)]
pub extern "C" fn kf3_trace_piece(h: *mut c_void, base: u64, len: u64) -> u32 {
    dev(h).map_or(0, |d| u32::from(d.trace.piece_traps(base, len)))
}

/// ★ ABI 24 (trace mode only; vCPU or main loop, lock-free): may the C device write this record?
/// `kind` 0 read / 1 write: `a` offset, `b` width, `c` value; 2 MSI: `a` vector, `b` data, `c`
/// address. 1 = write it; 0 = not selected, over a cap (counted as dropped), or the mode is off.
#[unsafe(no_mangle)]
pub extern "C" fn kf3_trace_admit(h: *mut c_void, kind: u32, a: u64, b: u64, c: u64) -> u32 {
    match (dev(h), crate::readtrace::Kind::from_abi(kind)) {
        (Some(d), Some(k)) => u32::from(d.trace.admit(k, a, b, c)),
        _ => 0,
    }
}

/// ★ ABI 24: the device name the trace records carry (the host GPU's PCI address unless
/// `KF3_TRACE_NAME` overrides it), NUL-terminated into `buf`.
///
/// # Safety
/// `buf` is null or writable for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kf3_trace_name(h: *mut c_void, buf: *mut c_char, len: usize) {
    if let Some(d) = dev(h) {
        write_err(buf, len, d.trace.name());
    }
}

/// ★ ABI 24: the trace's exit report (records, bytes, drops, cap hit), NUL-terminated into `buf`.
///
/// # Safety
/// `buf` is null or writable for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kf3_trace_report(h: *mut c_void, buf: *mut c_char, len: usize) {
    if let Some(d) = dev(h) {
        write_err(buf, len, &d.trace.report());
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

/// ★ ABI 23 (2026-10-08, `OWNER_RULINGS.md` §V; `docs/design/V3_DISPLAY.md` §8.20): one pointing
/// device the guest has, as the C device's pointers verb lists it (`InPointersFn`). ⊘ Replaces
/// ABI 12's `Kf3BrokerEvent` (the per-packet event array) and `Kf3Cursor`: the C device no longer
/// receives events to interpret — kf-broker calls its verbs ([`Kf3InputOps`]).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kf3Pointer {
    /// QEMU's mouse index (`qemu_mouse_set`).
    pub id: u32,
    /// 1: absolute (a tablet); 0: relative.
    pub absolute: u8,
    /// 1: a paravirtual (virtio-input) device.
    pub paravirtual: u8,
    /// Padding.
    pub pad: [u8; 2],
    /// The device's name, NUL-terminated or cut (the log only).
    pub name: [u8; 56],
}

impl Default for Kf3Pointer {
    fn default() -> Kf3Pointer {
        Kf3Pointer {
            id: 0,
            absolute: 0,
            paravirtual: 0,
            pad: [0; 2],
            name: [0; 56],
        }
    }
}

impl Kf3Pointer {
    /// The VMM-neutral device ([`kf_broker::PointerDevice`]); the name up to its NUL, lossily.
    #[must_use]
    pub fn device(&self) -> kf_broker::PointerDevice {
        let end = self
            .name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(self.name.len());
        kf_broker::PointerDevice {
            id: self.id,
            kind: if self.absolute != 0 {
                kf_broker::Pointer::Absolute
            } else {
                kf_broker::Pointer::Relative
            },
            paravirtual: self.paravirtual != 0,
            name: String::from_utf8_lossy(&self.name[..end]).into_owned(),
        }
    }
}

/// ★ ABI 23 (`OWNER_RULINGS.md` §V): the C device's input and console-cursor verbs — QEMU's half
/// of `kf_broker::InputSink` and `kf_broker::CursorSink`, handed over at [`kf3_broker_start`] and
/// copied there (every verb must be present). The verbs' contracts are their types'
/// (`crate::raw_unsafe`).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Kf3InputOps {
    /// A key edge.
    pub key: Option<crate::raw_unsafe::InKeyFn>,
    /// A button edge.
    pub button: Option<crate::raw_unsafe::InButtonFn>,
    /// One wheel detent.
    pub wheel: Option<crate::raw_unsafe::InWheelFn>,
    /// An absolute position.
    pub abs: Option<crate::raw_unsafe::InAbsFn>,
    /// Relative motion.
    pub rel: Option<crate::raw_unsafe::InRelFn>,
    /// End of a pointer report.
    pub sync: Option<crate::raw_unsafe::InSyncFn>,
    /// The pointing devices.
    pub pointers: Option<crate::raw_unsafe::InPointersFn>,
    /// Select a pointing device.
    pub select_pointer: Option<crate::raw_unsafe::InSelectFn>,
    /// A kind of pointing device is missing.
    pub missing_pointer: Option<crate::raw_unsafe::InMissingFn>,
    /// The window was closed.
    pub close: Option<crate::raw_unsafe::InCloseFn>,
    /// The window's size and refresh.
    pub resize_hint: Option<crate::raw_unsafe::InResizeFn>,
    /// The console cursor's image.
    pub cursor_define: Option<crate::raw_unsafe::CurDefineFn>,
    /// The console's hidden cursor.
    pub cursor_hide: Option<crate::raw_unsafe::CurHideFn>,
    /// The console cursor's position.
    pub cursor_move: Option<crate::raw_unsafe::CurMoveFn>,
    /// Whether the console's pointer is absolute.
    pub cursor_absolute: Option<crate::raw_unsafe::CurAbsoluteFn>,
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

/// ★ §8.13, ⊘ ABI 23 (`OWNER_RULINGS.md` §V, 2026-10-08; main thread: the console's `gfx_update`
/// AFTER it took its frame, and its refresh answer): bring QEMU's console cursor to the guest's —
/// while a cursor-capable broker hovers the frames carry no cursor (§O), so the console gets it
/// through QEMU's cursor API (VNC shows it as a real pointer); under grab, or with no such broker,
/// it stays composed. kf-broker decides what, when and whether (`kf_broker::ConsoleCursor::apply`)
/// and calls the C device's cursor verbs given at [`kf3_broker_start`]. Returns the number of
/// parts handed out (0: nothing to do, no display or no broker). ⊘ Replaces ABI 12's
/// `kf3_display_cursor` / `_pixels` / `_done`, whose define/hide/move decisions were the C
/// device's.
#[unsafe(no_mangle)]
pub extern "C" fn kf3_display_cursor_apply(h: *mut c_void) -> i32 {
    let Some(dp) = dev(h).and_then(|d| d.display) else {
        return 0;
    };
    let Some(seat) = dp.broker.as_ref() else {
        return 0;
    };
    let u = seat.apply_console_cursor(
        dp.console.cursor_point(),
        dp.console.shown_frame(),
        dp.console.now_ms(),
    );
    i32::from(u.define.is_some()) + i32::from(u.mouse.is_some())
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
/// inside the `kf3_broker_*` entries. ★ ABI 23 (`OWNER_RULINGS.md` §V, §8.20): `ops` is the C
/// device's input and console-cursor verbs (copied here; every one must be present), called with
/// the same `opaque` under the same rules. Returns 0, or -1 with a message (a broker that is not
/// running yet is NOT an error: the first attempt runs from the main loop's timer, and a failed
/// one is retried in the background).
///
/// # Safety
/// `path` is a NUL-terminated string; `ops` is null or readable; `err` is null or writable for
/// `err_len` bytes; `watch`, `timer` and every verb in `ops` must be callable with `opaque` on the
/// main loop for the device's lifetime, must not re-enter a `kf3_*` entry, and must not block.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kf3_broker_start(
    h: *mut c_void,
    path: *const c_char,
    extra_uid: i64,
    watch: Option<crate::raw_unsafe::BrokerWatchFn>,
    timer: Option<crate::raw_unsafe::BrokerTimerFn>,
    ops: *const Kf3InputOps,
    opaque: *mut c_void,
    now_ms: u64,
    err: *mut c_char,
    err_len: usize,
) -> i32 {
    let (Some(d), false, Some(watch), Some(timer), false) = (
        dev(h),
        path.is_null(),
        watch,
        timer,
        ops.is_null() || !ops.is_aligned(),
    ) else {
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
    // SAFETY: `ops` is non-null and aligned (checked above) and readable (caller contract); it is
    // copied, never kept.
    let ops = unsafe { *ops };
    // SAFETY: forwarded from this function's contract.
    let Some(sink) = (unsafe { crate::raw_unsafe::QemuSink::adopt(&ops, opaque) }) else {
        write_err(err, err_len, "kf3_broker_start: an input verb is missing");
        return -1;
    };
    // SAFETY: the caller promises a NUL-terminated string.
    let p = unsafe { CStr::from_ptr(path) };
    let path = std::path::PathBuf::from(std::ffi::OsStr::from_bytes(p.to_bytes()));
    // SAFETY: forwarded from this function's contract.
    let hooks = unsafe { crate::raw_unsafe::BrokerHooks::adopt(watch, timer, opaque) };
    match seat.start(&path, extra_uid, hooks, sink, now_ms) {
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
/// (`rd`/`wr` say which), the frame eventfd, or -1 for the relay's timer. Reads at most
/// `kf_broker::conn::READ_BATCH` packets from the socket (level-triggered readiness delivers the
/// rest). ★ ABI 23 (§8.20): the input is DELIVERED here, through the verbs given at
/// [`kf3_broker_start`] (`kf_broker::InputPolicy`), and the console's cursor follows (§8.13: every
/// cursor post is followed by a frame publish, which lands here). Returns the inputs delivered
/// (0 on a bad handle). ⊘ ABI 12 wrote `Kf3BrokerEvent`s for the C device to interpret.
#[unsafe(no_mangle)]
pub extern "C" fn kf3_broker_ready(h: *mut c_void, fd: i32, rd: u32, wr: u32, now_ms: u64) -> i32 {
    let Some(dp) = dev(h).and_then(|d| d.display) else {
        return 0;
    };
    let Some(seat) = dp.broker.as_ref() else {
        return 0;
    };
    let (active, n) = seat.ready(fd, rd != 0, wr != 0, now_ms);
    if active {
        // broker activity keeps the refresh clock at the watched rate; it asks for a host copy
        // only while the broker is fed through host memory (§8.11, two demand signals)
        dp.console.note_broker_demand();
    }
    // ★ §8.16: a session that just became active is a new viewer — the next check sends
    if seat.became_active(active) {
        dp.console.note_new_watcher();
    }
    seat.apply_console_cursor(
        dp.console.cursor_point(),
        dp.console.shown_frame(),
        dp.console.now_ms(),
    );
    i32::try_from(n).unwrap_or(i32::MAX)
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
