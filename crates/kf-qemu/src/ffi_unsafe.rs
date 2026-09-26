//! ★ The `extern "C"` surface the kf3-gpu QOM device calls. Every entry point validates its handle
//! and pointers and hands off to the safe [`crate::device::Device`] immediately.

use crate::device::{Config, Device};
use crate::raw_unsafe::RawRegion;
use core::ffi::{c_char, c_void};
use std::ffi::CStr;

/// Wire ABI of this surface; the C device refuses a mismatched archive.
pub const KF3_ABI: u32 = 5;

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
    out: *mut *mut c_void,
    err: *mut c_char,
    err_len: usize,
) -> i32 {
    let guest = if guest_driver.is_null() {
        None
    } else {
        // SAFETY: the caller promises a NUL-terminated string.
        Some(unsafe { CStr::from_ptr(guest_driver) }.to_string_lossy().into_owned()).filter(|s| !s.is_empty())
    };
    let cfg = Config { gpu_minor, fb_mb, bar1_bytes, bar2_bytes, guest_driver: guest };
    match Device::realize(&cfg) {
        Ok(d) => {
            let d: &'static Device = Box::leak(Box::new(d));
            if std::thread::Builder::new().name("kf3-drainer".into()).spawn(move || d.drainer_loop()).is_err() {
                write_err(err, err_len, "could not start the register drainer thread");
                return -1;
            }
            // ★ P5: the workers — they serve rung Translated tokens and host completions.
            for i in 0..2 {
                if std::thread::Builder::new().name(format!("kf3-worker{i}")).spawn(move || d.worker_loop()).is_err() {
                    write_err(err, err_len, "could not start a worker thread");
                    return -1;
                }
            }
            // ★ P4: the VA-manager thread — the one owner of the GPU walker.
            if std::thread::Builder::new().name("kf3-vamgr".into()).spawn(move || d.va_loop()).is_err() {
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
    let (Some(d), false) = (dev(h), out.is_null()) else { return -1 };
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

/// The memory map: writes up to `cap` regions to `out`, returns the total count.
///
/// # Safety
/// `out` is null or writable for `cap` regions.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kf3_memory_map(h: *mut c_void, bar1: u64, bar2: u64, out: *mut Kf3Region, cap: usize) -> i64 {
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
pub unsafe extern "C" fn kf3_shadow_attach(h: *mut c_void, base: u64, mem: *mut u8, len: u64) -> i32 {
    let (Some(d), false) = (dev(h), mem.is_null()) else { return -1 };
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
pub unsafe extern "C" fn kf3_ram_add(h: *mut c_void, gpa: u64, hva: *mut u8, len: u64, fd: i32, fd_off: u64) -> i32 {
    let (Some(d), false) = (dev(h), hva.is_null()) else { return -1 };
    // SAFETY: the caller keeps the RAM mapped until it unregisters it.
    // SAFETY: the caller keeps the RAM mapped, and its backend fd open, until it unregisters it.
    let (mem, fd) = unsafe { (RawRegion::adopt(hva, len as usize), crate::raw_unsafe::BackendFd::adopt(fd)) };
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
pub unsafe extern "C" fn kf3_bar_ram(h: *mut c_void, bar: u32, base: u64, len: u64, ptr: *mut *mut c_void) -> i32 {
    let (Some(d), false) = (dev(h), ptr.is_null()) else { return -1 };
    let Some(span) = d.window_address(bar, base, len) else { return -1 };
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
pub unsafe extern "C" fn kf3_usermode_view(h: *mut c_void, ptr: *mut *mut c_void, len: *mut u64) -> i32 {
    let Some(d) = dev(h) else { return -1 };
    let Ok(span) = d.rm.usermode_view() else { return -1 };
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

/// ★ Register the C device's BAR1 overlay verb (`V3_BAR1_DOORBELL.md` §4). Returns 0, or -1 (bad
/// handle, null verb, or already registered).
///
/// # Safety
/// `f` must be callable from any non-vCPU thread with `opaque` for the process's lifetime; it
/// may block (bounded) and must never be called on a vCPU.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kf3_set_bar1_overlay(h: *mut c_void, f: Option<crate::raw_unsafe::OverlayFn>, opaque: *mut c_void) -> i32 {
    let (Some(d), Some(f)) = (dev(h), f) else { return -1 };
    // SAFETY: forwarded from this function's contract.
    let hook = unsafe { crate::raw_unsafe::OverlayHook::adopt(f, opaque) };
    if d.bar1_overlay.set(hook) { 0 } else { -1 }
}

/// ★ THE vCPU PATH for a write into a BAR1 usermode overlay: `vf_rel` = offset inside the 64 KiB
/// usermode page (the overlay aliases it, so QEMU hands us that offset directly). Lock-free.
#[unsafe(no_mangle)]
pub extern "C" fn kf3_bar1_usermode_write(h: *mut c_void, vf_rel: u64, val: u64, width: u32) {
    if let Some(d) = dev(h) {
        d.bar1_usermode_write(vf_rel, val, u8::try_from(width).unwrap_or(4));
    }
}

/// Stop the device's threads (the device itself lives for the process).
#[unsafe(no_mangle)]
pub extern "C" fn kf3_unrealize(h: *mut c_void) {
    if let Some(d) = dev(h) {
        d.stop();
    }
}
