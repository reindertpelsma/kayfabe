// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! The checks. Each prints `KFGOP-TEST check <name> ok|FAIL <detail>`; the verdict line counts them.

use core::ffi::c_void;
use core::fmt;
use core::mem::size_of;

use kf_gop::test_pattern;
use kf_oprom::desc::Descriptor;

use crate::efi_unsafe::{
    Bs, DEVICE_PATH_GUID, DevicePath, EDID_ACTIVE_GUID, EDID_DISCOVERED_GUID, Edid, GOP_GUID, Gop,
    Handle, INVALID_PARAMETER, ModeInfo, PCI_IO_GUID, PciIo, SUCCESS, Status, SystemTable,
    UNSUPPORTED,
};

macro_rules! log {
    ($($t:tt)*) => {
        crate::port_unsafe::write(kf_gop::log::line(format_args!($($t)*)).bytes())
    };
}

struct Tally {
    n: u32,
    failed: u32,
}

impl Tally {
    fn check(&mut self, name: &str, ok: bool, detail: fmt::Arguments<'_>) {
        self.n += 1;
        if !ok {
            self.failed += 1;
        }
        log!(
            "KFGOP-TEST check {} {} {}",
            name,
            if ok { "ok" } else { "FAIL" },
            detail
        );
    }
}

const FILL: u32 = 0x0012_3456;

/// The GOP kf-gop published, with what the device's ROM says it should be.
struct Found<'a> {
    gop: *mut Gop,
    handle: Handle,
    desc: Descriptor<'a>,
    bar_base: u64,
    bar_len: u64,
}

fn bar_range(bs: Bs, pci: *mut PciIo, bar: u8) -> Option<(u64, u64)> {
    let mut res: *mut c_void = core::ptr::null_mut();
    // SAFETY: `pci` came from `HandleProtocol` on a PCI handle; `res` is a local the call fills with a
    // pool buffer of ACPI descriptors.
    let s = unsafe { ((*pci).get_bar_attributes)(pci, bar, core::ptr::null_mut(), &mut res) };
    if crate::efi_unsafe::is_error(s) || res.is_null() {
        return None;
    }
    // SAFETY: a first tag of 0x8A is a 46-byte QWORD descriptor, which is all that is read.
    let r = unsafe {
        let p = res.cast::<u8>();
        (*p == 0x8A)
            .then(|| kf_gop::probe::bar_range(core::slice::from_raw_parts(p, 46)).ok())
            .flatten()
    };
    // SAFETY: `GetBarAttributes` allocated `res` for its caller to free.
    unsafe { bs.free_pool(res) };
    r
}

/// Every GOP handle with a device path whose PCI parent's ROM carries a descriptor.
fn find<'a>(bs: Bs) -> (usize, usize, Option<Found<'a>>) {
    let Ok((handles, n)) = bs.locate_handle_buffer(&GOP_GUID) else {
        return (0, 0, None);
    };
    let mut ours = 0;
    let mut found = None;
    for i in 0..n {
        // SAFETY: `LocateHandleBuffer` returned `n` handles at `handles`.
        let h = unsafe { *handles.add(i) };
        let Ok(dp) = bs.handle_protocol(h, &DEVICE_PATH_GUID) else {
            continue; // the console splitter's GOP has no device path
        };
        // SAFETY: `dp` is the device-path protocol instance of `h`, a well-formed path.
        let Ok(pci_handle) =
            (unsafe { bs.locate_device_path(&PCI_IO_GUID, dp.cast::<DevicePath>()) })
        else {
            continue;
        };
        let Ok(pci) = bs.handle_protocol(pci_handle, &PCI_IO_GUID) else {
            continue;
        };
        let pci = pci.cast::<PciIo>();
        // SAFETY: `RomImage`/`RomSize` describe the PCI bus driver's copy of the ROM, alive as long as
        // the device; the slice is read only.
        let rom: &'a [u8] = unsafe {
            let (p, len) = ((*pci).rom_image, (*pci).rom_size);
            if p.is_null() || len == 0 {
                continue;
            }
            core::slice::from_raw_parts(p.cast::<u8>(), len as usize)
        };
        let Ok(desc) = Descriptor::find(rom) else {
            continue;
        };
        // Ours only if the descriptor names this controller, as the driver itself requires: a ROM
        // whose ids do not match is refused by kf-gop, and another driver may own the device.
        let mut id = 0u32;
        // SAFETY: `pci` came from `HandleProtocol`; one 32-bit config read into the local `id`.
        let s = unsafe { ((*pci).pci.read)(pci, 2, 0, 1, (&raw mut id).cast()) };
        if crate::efi_unsafe::is_error(s)
            || (id as u16, (id >> 16) as u16) != (desc.vendor, desc.device)
        {
            log!(
                "KFGOP-TEST rom_ids_mismatch config={:04x}:{:04x} descriptor={:04x}:{:04x}",
                id as u16,
                (id >> 16) as u16,
                desc.vendor,
                desc.device
            );
            continue;
        }
        ours += 1;
        let Some((bar_base, bar_len)) = bar_range(bs, pci, desc.fb.bar) else {
            continue;
        };
        let Ok(gop) = bs.handle_protocol(h, &GOP_GUID) else {
            continue;
        };
        found = Some(Found {
            gop: gop.cast(),
            handle: h,
            desc,
            bar_base,
            bar_len,
        });
    }
    // SAFETY: the array came from `LocateHandleBuffer`, which leaves freeing to the caller.
    unsafe { bs.free_pool(handles.cast()) };
    (n, ours, found)
}

struct Screen {
    gop: *mut Gop,
    fb: u64,
    pitch: usize,
}

impl Screen {
    #[allow(clippy::too_many_arguments)]
    fn blt(
        &self,
        buf: *mut u8,
        op: u32,
        sx: usize,
        sy: usize,
        dx: usize,
        dy: usize,
        w: usize,
        h: usize,
        delta: usize,
    ) -> Status {
        // SAFETY: `gop` is the live GOP instance found above; every caller passes a buffer covering the
        // rectangle it names (or null where the call is meant to be refused).
        unsafe { ((*self.gop).blt)(self.gop, buf, op, sx, sy, dx, dy, w, h, delta) }
    }

    /// One pixel straight from the framebuffer, as BGRX packed little-endian.
    fn fb_pixel(&self, x: usize, y: usize) -> u32 {
        // SAFETY: callers stay inside the mode, so the address is inside `[FrameBufferBase,
        // FrameBufferBase + pitch·height)`, which the GOP publishes as readable memory (identity-mapped).
        unsafe { ((self.fb as usize + y * self.pitch + x * 4) as *const u32).read_volatile() }
    }
}

fn px(p: [u8; 4]) -> u32 {
    u32::from_le_bytes(p)
}

/// A pool buffer of `w·h` pixels.
struct Pixels {
    bs: Bs,
    p: *mut u32,
    len: usize,
}

impl Pixels {
    fn new(bs: Bs, w: usize, h: usize) -> Option<Pixels> {
        let len = w * h;
        let p = bs.allocate_pool(len * 4).ok()?.cast::<u32>();
        Some(Pixels { bs, p, len })
    }
    fn slice(&mut self) -> &mut [u32] {
        // SAFETY: `p` is this value's pool allocation of `len` u32s (pool memory is 8-byte aligned).
        unsafe { core::slice::from_raw_parts_mut(self.p, self.len) }
    }
    fn raw(&self) -> *mut u8 {
        self.p.cast()
    }
}

impl Drop for Pixels {
    fn drop(&mut self) {
        // SAFETY: `p` is this value's own pool allocation, not used after drop.
        unsafe { self.bs.free_pool(self.p.cast()) };
    }
}

fn edid_matches(bs: Bs, h: Handle, guid: &'static crate::efi_unsafe::Guid, want: &[u8]) -> bool {
    let Ok(e) = bs.handle_protocol(h, guid) else {
        return false;
    };
    let e = e.cast::<Edid>();
    // SAFETY: the EDID protocol instance holds `SizeOfEdid` bytes at `Edid`.
    unsafe {
        let (n, p) = ((*e).size_of_edid as usize, (*e).edid);
        n == want.len() && !p.is_null() && core::slice::from_raw_parts(p, n) == want
    }
}

/// The application entry point.
#[unsafe(no_mangle)]
pub extern "efiapi" fn efi_main(_image: Handle, st: *mut SystemTable) -> Status {
    // SAFETY: the firmware passes its system table to the entry point.
    let bs = unsafe { Bs::from_system_table(st) };
    bs.disable_watchdog();
    log!("KFGOP-TEST start");
    // SAFETY: the firmware passes its system table to the entry point.
    match unsafe { crate::efi_unsafe::secure_boot(st) } {
        Ok(v) => log!("KFGOP-TEST secure_boot={}", v),
        Err(s) if s == crate::efi_unsafe::NOT_FOUND => log!("KFGOP-TEST secure_boot=absent"),
        Err(s) => log!("KFGOP-TEST secure_boot=unreadable status={:#x}", s),
    }
    let mut t = Tally { n: 0, failed: 0 };
    let (total, ours, found) = find(bs);
    log!("KFGOP-TEST gop_handles total={} ours={}", total, ours);
    t.check("one_kf_gop", ours == 1, format_args!("ours={ours}"));
    let Some(f) = found else {
        log!(
            "KFGOP-TEST RESULT FAIL checks={} failed={} reason=no-kf-gop",
            t.n,
            t.failed + 1
        );
        loop {
            bs.stall(1_000_000);
        }
    };
    let g = f.desc.fb.geometry;
    let (w, h) = (g.width as usize, g.height as usize);
    // SAFETY: `f.gop` is a live GOP instance and its `Mode` block is published memory.
    let (mode, info) = unsafe { (&*(*f.gop).mode, *(*(*f.gop).mode).info) };
    log!(
        "KFGOP-TEST mode width={} height={} ppsl={} format={} fb_base={:#x} fb_size={:#x} bar={} bar_base={:#x} bar_len={:#x}",
        info.horizontal_resolution,
        info.vertical_resolution,
        info.pixels_per_scan_line,
        info.pixel_format,
        mode.frame_buffer_base,
        mode.frame_buffer_size,
        f.desc.fb.bar,
        f.bar_base,
        f.bar_len
    );
    t.check(
        "one_mode",
        mode.max_mode == 1 && mode.mode == 0,
        format_args!("max={} cur={}", mode.max_mode, mode.mode),
    );
    t.check(
        "mode_is_the_descriptors",
        info.horizontal_resolution == g.width
            && info.vertical_resolution == g.height
            && info.pixels_per_scan_line == g.pitch / 4
            && info.pixel_format == kf_gop::PIXEL_BGRX
            && mode.size_of_info == size_of::<ModeInfo>(),
        format_args!("pitch={}", g.pitch),
    );
    t.check(
        "framebuffer_is_the_bar",
        mode.frame_buffer_base == f.bar_base + f.desc.fb.offset
            && mode.frame_buffer_size as u64 == g.fb_size,
        format_args!(
            "base={:#x} want={:#x}",
            mode.frame_buffer_base,
            f.bar_base + f.desc.fb.offset
        ),
    );
    t.check(
        "edid_discovered",
        edid_matches(bs, f.handle, &EDID_DISCOVERED_GUID, f.desc.edid),
        format_args!("{} bytes", f.desc.edid.len()),
    );
    t.check(
        "edid_active",
        edid_matches(bs, f.handle, &EDID_ACTIVE_GUID, f.desc.edid),
        format_args!("{} bytes", f.desc.edid.len()),
    );

    // QueryMode
    let mut size = 0usize;
    let mut qi: *mut ModeInfo = core::ptr::null_mut();
    // SAFETY: live GOP; `size` and `qi` are locals.
    let s = unsafe { ((*f.gop).query_mode)(f.gop, 0, &mut size, &mut qi) };
    // SAFETY: on success `qi` is a pool `ModeInfo` the caller owns.
    let same = s == SUCCESS
        && size == size_of::<ModeInfo>()
        && !qi.is_null()
        && unsafe {
            let q = *qi;
            q.horizontal_resolution == g.width
                && q.vertical_resolution == g.height
                && q.pixels_per_scan_line == g.pitch / 4
        };
    if !qi.is_null() {
        // SAFETY: `QueryMode` allocated it for us.
        unsafe { bs.free_pool(qi.cast()) };
    }
    t.check(
        "query_mode_0",
        same,
        format_args!("status={s:#x} size={size}"),
    );
    // SAFETY: live GOP; out-pointers are locals.
    let s = unsafe { ((*f.gop).query_mode)(f.gop, 1, &mut size, &mut qi) };
    t.check(
        "query_mode_1_invalid",
        s == INVALID_PARAMETER,
        format_args!("status={s:#x}"),
    );
    // SAFETY: live GOP.
    let s = unsafe { ((*f.gop).set_mode)(f.gop, 1) };
    t.check(
        "set_mode_1_unsupported",
        s == UNSUPPORTED,
        format_args!("status={s:#x}"),
    );

    let scr = Screen {
        gop: f.gop,
        fb: mode.frame_buffer_base,
        pitch: g.pitch as usize,
    };

    // Fill, read back through Blt and straight from the framebuffer.
    let mut one = [FILL];
    let s = scr.blt(one.as_mut_ptr().cast(), 0, 0, 0, 8, 8, 40, 30, 0);
    let Some(mut back) = Pixels::new(bs, 40, 30) else {
        log!("KFGOP-TEST RESULT FAIL reason=pool");
        loop {
            bs.stall(1_000_000);
        }
    };
    let s2 = scr.blt(back.raw(), 1, 8, 8, 0, 0, 40, 30, 0);
    let ok = s == SUCCESS && s2 == SUCCESS && back.slice().iter().all(|p| *p == FILL);
    t.check(
        "fill_and_read_back",
        ok,
        format_args!("fill={s:#x} read={s2:#x}"),
    );
    let ok =
        scr.fb_pixel(8, 8) == FILL && scr.fb_pixel(47, 37) == FILL && scr.fb_pixel(48, 37) != FILL;
    t.check(
        "fill_reaches_framebuffer",
        ok,
        format_args!("fb(8,8)={:#x}", scr.fb_pixel(8, 8)),
    );

    // BufferToVideo, tight buffer.
    if let (Some(mut src), Some(mut rd)) = (Pixels::new(bs, 64, 32), Pixels::new(bs, 64, 32)) {
        for (i, p) in src.slice().iter_mut().enumerate() {
            *p = px(test_pattern((i % 64) as u32, (i / 64) as u32));
        }
        let s = scr.blt(src.raw(), 2, 0, 0, 100, 100, 64, 32, 0);
        let s2 = scr.blt(rd.raw(), 1, 100, 100, 0, 0, 64, 32, 0);
        let ok = s == SUCCESS && s2 == SUCCESS && src.slice() == rd.slice();
        t.check(
            "buffer_to_video_round_trip",
            ok,
            format_args!("write={s:#x} read={s2:#x}"),
        );
        let mut fb_ok = true;
        for y in 0..32 {
            for x in 0..64 {
                fb_ok &= scr.fb_pixel(100 + x, 100 + y) == px(test_pattern(x as u32, y as u32));
            }
        }
        t.check(
            "buffer_to_video_reaches_framebuffer",
            fb_ok,
            format_args!("64x32 at (100,100)"),
        );
    }

    // Delta and source offsets.
    if let (Some(mut src), Some(mut rd)) = (Pixels::new(bs, 96, 48), Pixels::new(bs, 50, 20)) {
        for (i, p) in src.slice().iter_mut().enumerate() {
            *p = px(test_pattern((i % 96) as u32 + 500, (i / 96) as u32 + 300));
        }
        let s = scr.blt(src.raw(), 2, 10, 5, 300, 200, 50, 20, 96 * 4);
        let s2 = scr.blt(rd.raw(), 1, 300, 200, 0, 0, 50, 20, 0);
        let mut ok = s == SUCCESS && s2 == SUCCESS;
        for (i, p) in rd.slice().iter().enumerate() {
            ok &= *p == px(test_pattern((i % 50) as u32 + 510, (i / 50) as u32 + 305));
        }
        t.check(
            "delta_and_source_offset",
            ok,
            format_args!("write={s:#x} read={s2:#x}"),
        );
    }

    // VideoToVideo, overlapping: the 64x32 pattern at (100,100) moves to (120,108).
    if let Some(mut rd) = Pixels::new(bs, 64, 32) {
        let s = scr.blt(core::ptr::null_mut(), 3, 100, 100, 120, 108, 64, 32, 0);
        let s2 = scr.blt(rd.raw(), 1, 120, 108, 0, 0, 64, 32, 0);
        let mut ok = s == SUCCESS && s2 == SUCCESS;
        for (i, p) in rd.slice().iter().enumerate() {
            let (x, y) = ((i % 64) as u32, (i / 64) as u32);
            ok &= *p == px(test_pattern(x, y))
                && scr.fb_pixel(120 + x as usize, 108 + y as usize) == *p;
        }
        t.check(
            "video_to_video_overlap",
            ok,
            format_args!("move={s:#x} read={s2:#x}"),
        );
    }

    // Refusals.
    let s = scr.blt(one.as_mut_ptr().cast(), 0, 0, 0, 0, 0, 0, 1, 0);
    t.check(
        "refuse_zero_width",
        s == INVALID_PARAMETER,
        format_args!("status={s:#x}"),
    );
    let s = scr.blt(one.as_mut_ptr().cast(), 0, 0, 0, w - 10, 0, 20, 1, 0);
    t.check(
        "refuse_off_screen",
        s == INVALID_PARAMETER,
        format_args!("status={s:#x}"),
    );
    let s = scr.blt(one.as_mut_ptr().cast(), 4, 0, 0, 0, 0, 1, 1, 0);
    t.check(
        "refuse_unknown_op",
        s == INVALID_PARAMETER,
        format_args!("status={s:#x}"),
    );
    let s = scr.blt(core::ptr::null_mut(), 0, 0, 0, 0, 0, 1, 1, 0);
    t.check(
        "refuse_fill_without_pixel",
        s == INVALID_PARAMETER,
        format_args!("status={s:#x}"),
    );

    // SetMode(0) clears to black, in the framebuffer and the shadow.
    // SAFETY: live GOP.
    let s = unsafe { ((*f.gop).set_mode)(f.gop, 0) };
    let mut p = [0xFFFF_FFFFu32];
    let s2 = scr.blt(p.as_mut_ptr().cast(), 1, 8, 8, 0, 0, 1, 1, 0);
    let ok = s == SUCCESS
        && s2 == SUCCESS
        && p[0] == 0
        && scr.fb_pixel(8, 8) == 0
        && scr.fb_pixel(w - 1, h - 1) == 0;
    t.check(
        "set_mode_0_clears",
        ok,
        format_args!(
            "status={s:#x} shadow={:#x} fb={:#x}",
            p[0],
            scr.fb_pixel(8, 8)
        ),
    );

    // The full-screen pattern the host compares.
    let drawn = match Pixels::new(bs, w, h) {
        Some(mut all) => {
            for (i, p) in all.slice().iter_mut().enumerate() {
                *p = px(test_pattern((i % w) as u32, (i / w) as u32));
            }
            scr.blt(all.raw(), 2, 0, 0, 0, 0, w, h, 0)
        }
        None => crate::efi_unsafe::OUT_OF_RESOURCES,
    };
    let ok = drawn == SUCCESS
        && scr.fb_pixel(0, 0) == px(test_pattern(0, 0))
        && scr.fb_pixel(w - 1, h - 1) == px(test_pattern(w as u32 - 1, h as u32 - 1));
    t.check("full_screen_pattern", ok, format_args!("status={drawn:#x}"));

    log!(
        "KFGOP-TEST RESULT {} checks={} failed={}",
        if t.failed == 0 { "PASS" } else { "FAIL" },
        t.n,
        t.failed
    );
    log!(
        "KFGOP-TEST PATTERN fb_base={:#x} width={} height={} pitch={}",
        mode.frame_buffer_base,
        w,
        h,
        g.pitch
    );
    log!("KFGOP-TEST READY");
    loop {
        bs.stall(1_000_000);
    }
}
