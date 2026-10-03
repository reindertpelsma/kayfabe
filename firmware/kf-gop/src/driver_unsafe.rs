// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! The driver's entry points: the image entry, the driver binding (`Supported`/`Start`/`Stop`),
//! component name, and the GOP (`QueryMode`/`SetMode`/`Blt`).
//!
//! The UEFI driver model, as this driver applies it:
//! - `efi_main` installs the driver binding and component name on its own image handle and returns.
//!   Nothing is touched until the firmware connects a controller.
//! - `Supported` opens the controller's PCI I/O `BY_DRIVER`, asks [`probe::decide`], closes it.
//! - `Start` opens PCI I/O `BY_DRIVER` for keeps, enables `EFI_PCI_IO_ATTRIBUTE_MEMORY` (saving the
//!   previous attributes), places the framebuffer inside the BAR the bus driver assigned, allocates a
//!   context and a RAM shadow, creates one child handle (the parent's device path plus an ACPI `_ADR`
//!   node, as EDK2's own video drivers do), installs device path, GOP and both EDID protocols on it,
//!   opens PCI I/O `BY_CHILD_CONTROLLER` from it, and clears the screen (`SetMode(0)`).
//! - `Stop` reverses it child by child, then closes the controller.
//!
//! Every GOP call runs at `TPL_NOTIFY`, so a timer callback cannot re-enter `Blt` while the shadow is
//! being written.

use core::ffi::c_void;
use core::mem::size_of;

use kf_gop::blt::{self, Access, BPP, Buffer, Framebuffer, Invalid, Surface};
use kf_gop::probe::{self, Boot, Config, Refusal};
use kf_oprom::desc::EDID_MAX;

use crate::efi_unsafe::{
    ATTR_ENABLE, ATTR_GET, ATTR_MEMORY, ATTR_SET, ATTR_SUPPORTED, Bs, COMPONENT_NAME2_GUID,
    ComponentName2, DEVICE_PATH_GUID, DP_ACPI, DP_ACPI_ADR, DP_END, DP_END_NODE,
    DRIVER_BINDING_GUID, DevicePath, DriverBinding, EDID_ACTIVE_GUID, EDID_DISCOVERED_GUID, Edid,
    GOP_GUID, Gop, GopMode, Handle, INVALID_PARAMETER, ModeInfo, OPEN_BY_CHILD_CONTROLLER,
    OPEN_BY_DRIVER, OPEN_GET_PROTOCOL, OUT_OF_RESOURCES, PciIo, SUCCESS, Status, SystemTable,
    TPL_NOTIFY, UNSUPPORTED, is_error,
};

/// A debug line on port 0x402, in `debugcon` builds only. Without the feature the arguments are
/// still type-checked (inside a closure that is never called), so a release build cannot rot them.
macro_rules! debug {
    ($($t:tt)*) => {{
        #[cfg(feature = "debugcon")]
        crate::port_unsafe::write(kf_gop::log::line(format_args!($($t)*)).bytes());
        #[cfg(not(feature = "debugcon"))]
        let _ = || kf_gop::log::line(format_args!($($t)*));
    }};
}

const fn utf16<const N: usize>(s: &[u8; N]) -> [u16; N] {
    let mut o = [0u16; N];
    let mut i = 0;
    while i < N {
        o[i] = s[i] as u16;
        i += 1;
    }
    o
}

static DRIVER_NAME: [u16; 16] = utf16(b"kayfabe kf3 GOP\0");
static LANGUAGES: [u8; 3] = *b"en\0";

/// What `efi_main` allocates once: the driver binding first, so its `This` is the whole.
#[repr(C)]
struct Driver {
    binding: DriverBinding,
    name: ComponentName2,
    bs: Bs,
}

const CTX_SIGNATURE: u64 = u64::from_le_bytes(*b"kf3GOPv1");
const PAGE: usize = 4096;
/// `_ADR` for the child: `ACPI_DISPLAY_ADR(1, 0, 0, 1, 0, ACPI_ADR_DISPLAY_TYPE_VGA, 0, 0)`, the value
/// EDK2's video drivers use for their one output.
const ACPI_ADR_VGA: u32 = (1 << 31) | (1 << 16) | (1 << 8);

/// One started controller. The GOP comes first, so a GOP call's `This` is the context.
#[repr(C)]
struct Ctx {
    gop: Gop,
    signature: u64,
    bs: Bs,
    image: Handle,
    controller: Handle,
    child: Handle,
    pci: *mut PciIo,
    saved_attrs: u64,
    mode: GopMode,
    info: ModeInfo,
    edid_discovered: Edid,
    edid_active: Edid,
    edid: [u8; EDID_MAX],
    device_path: *mut DevicePath,
    device_path_bytes: usize,
    shadow: u64,
    shadow_pages: usize,
    fb_base: u64,
    surface: Surface,
}

/// The image entry point.
#[unsafe(no_mangle)]
pub extern "efiapi" fn efi_main(image: Handle, st: *mut SystemTable) -> Status {
    core::hint::black_box(kf_gop::ABI_MARKER);
    // SAFETY: the firmware passes its system table to the entry point.
    let bs = unsafe { Bs::from_system_table(st) };
    debug!("kf-gop: loaded");
    let d = match bs.allocate_pool(size_of::<Driver>()) {
        Ok(p) => p.cast::<Driver>(),
        Err(s) => return s,
    };
    let driver = Driver {
        binding: DriverBinding {
            supported,
            start,
            stop,
            version: 0x10,
            image_handle: image,
            driver_binding_handle: image,
        },
        name: ComponentName2 {
            get_driver_name,
            get_controller_name,
            supported_languages: LANGUAGES.as_ptr(),
        },
        bs,
    };
    // SAFETY: `d` is a fresh pool allocation of `size_of::<Driver>()` bytes; pool memory is 8-byte
    // aligned, which is `Driver`'s alignment.
    unsafe { d.write(driver) };
    let mut h = image;
    // SAFETY: `d` is never freed (the image has no unload), so both interfaces outlive their
    // installation; each points at a structure of its protocol's layout.
    let s = unsafe { bs.install(&mut h, &DRIVER_BINDING_GUID, d.cast()) };
    if is_error(s) {
        return s;
    }
    // SAFETY: as above; `&raw mut` takes the field's address without creating a reference.
    let name = unsafe { &raw mut (*d).name };
    // SAFETY: as above.
    let _ = unsafe { bs.install(&mut h, &COMPONENT_NAME2_GUID, name.cast()) };
    SUCCESS
}

/// The `Driver` a driver-binding call is about.
fn driver<'a>(this: *mut DriverBinding) -> &'a Driver {
    // SAFETY: the firmware calls binding functions with the interface pointer we installed, which is
    // the first field of a live, never-freed `#[repr(C)]` `Driver`.
    unsafe { &*this.cast::<Driver>() }
}

/// `RemainingDevicePath` is acceptable when absent, the end node, or our `_ADR` node.
fn remaining_ok(remaining: *mut DevicePath) -> bool {
    if remaining.is_null() {
        return true;
    }
    // SAFETY: a non-null `RemainingDevicePath` points at a well-formed device-path node (UEFI 2.x
    // `Supported()`), and a node header is three bytes.
    let (ty, sub) = unsafe { ((*remaining).ty, (*remaining).sub_type) };
    ty == DP_END || (ty == DP_ACPI && sub == DP_ACPI_ADR)
}

fn remaining_is_end(remaining: *mut DevicePath) -> bool {
    // SAFETY: as in `remaining_ok`.
    !remaining.is_null() && unsafe { (*remaining).ty } == DP_END
}

fn config_dword(pci: *mut PciIo, offset: u32) -> Option<u32> {
    let mut v = 0u32;
    // SAFETY: `pci` is a PCI I/O instance the firmware handed us (opened by this driver); one 32-bit
    // read (width 2 = `EfiPciIoWidthUint32`, count 1) writes four bytes into the local `v`.
    let s = unsafe { ((*pci).pci.read)(pci, 2, offset, 1, (&raw mut v).cast()) };
    (!is_error(s)).then_some(v)
}

/// [`probe::decide`] over this controller's config space and ROM image.
fn examine(pci: *mut PciIo) -> Result<Boot, Refusal> {
    let cfg = match (config_dword(pci, 0x00), config_dword(pci, 0x08)) {
        (Some(id), Some(class)) => Config::from_dwords(id, class),
        _ => return Err(Refusal::ConfigUnreadable),
    };
    // SAFETY: `pci` is an opened PCI I/O instance. `RomImage`/`RomSize` describe the bus driver's
    // in-memory copy of the option ROM, which lives as long as the instance; the slice is used only
    // inside this call (`decide` copies what it keeps).
    let rom = unsafe {
        let (p, n) = ((*pci).rom_image, (*pci).rom_size);
        (!p.is_null() && n != 0).then(|| core::slice::from_raw_parts(p.cast::<u8>(), n as usize))
    };
    probe::decide(&cfg, rom)
}

fn attributes(pci: *mut PciIo, op: u32, attrs: u64) -> Result<u64, Status> {
    let mut out = 0u64;
    // SAFETY: `pci` is an opened PCI I/O instance; `out` is a local the call may write.
    let s = unsafe { ((*pci).attributes)(pci, op, attrs, &mut out) };
    if is_error(s) { Err(s) } else { Ok(out) }
}

/// `(base, length)` of BAR `bar`, from `GetBarAttributes`.
fn bar_range(bs: Bs, pci: *mut PciIo, bar: u8) -> Result<(u64, u64), Refusal> {
    let mut res: *mut c_void = core::ptr::null_mut();
    // SAFETY: `pci` is an opened PCI I/O instance; `res` is a local the call sets to a pool buffer.
    let s = unsafe { ((*pci).get_bar_attributes)(pci, bar, core::ptr::null_mut(), &mut res) };
    if is_error(s) || res.is_null() {
        return Err(Refusal::BarNotMemory);
    }
    let first = res.cast::<u8>();
    // SAFETY: `GetBarAttributes` returns ACPI resource descriptors ending in an end tag. A first tag
    // of 0x8A is a QWORD descriptor of exactly `ACPI_QWORD_LEN` bytes, which is all that is read.
    let r = unsafe {
        if *first == 0x8A {
            probe::bar_range(core::slice::from_raw_parts(first, probe::ACPI_QWORD_LEN))
        } else {
            Err(Refusal::BarNotMemory)
        }
    };
    // SAFETY: the buffer is the pool allocation `GetBarAttributes` made for its caller to free.
    unsafe { bs.free_pool(res) };
    r
}

extern "efiapi" fn supported(
    this: *mut DriverBinding,
    controller: Handle,
    remaining: *mut DevicePath,
) -> Status {
    let d = driver(this);
    let image = d.binding.image_handle;
    let pci = match d.bs.open_protocol(
        controller,
        &crate::efi_unsafe::PCI_IO_GUID,
        image,
        controller,
        OPEN_BY_DRIVER,
    ) {
        Ok(p) => p.cast::<PciIo>(),
        Err(s) => {
            // UNSUPPORTED is every handle without PCI I/O; anything else (ACCESS_DENIED: another
            // driver owns the device) is worth a line.
            if s != UNSUPPORTED {
                debug!("kf-gop: Supported cannot open PCI I/O, status {:#x}", s);
            }
            return s;
        }
    };
    let r = examine(pci);
    d.bs.close_protocol(
        controller,
        &crate::efi_unsafe::PCI_IO_GUID,
        image,
        controller,
    );
    match r {
        Ok(_) if !remaining_ok(remaining) => {
            debug!("kf-gop: Supported refuses: a remaining device path that is not ours");
            UNSUPPORTED
        }
        Ok(_) => {
            debug!("kf-gop: Supported accepts");
            SUCCESS
        }
        Err(why) => {
            if !matches!(why, Refusal::NotDisplay(_)) {
                debug!("kf-gop: Supported refuses: {}", why);
            }
            UNSUPPORTED
        }
    }
}

extern "efiapi" fn start(
    this: *mut DriverBinding,
    controller: Handle,
    remaining: *mut DevicePath,
) -> Status {
    let d = driver(this);
    let image = d.binding.image_handle;
    let pci = match d.bs.open_protocol(
        controller,
        &crate::efi_unsafe::PCI_IO_GUID,
        image,
        controller,
        OPEN_BY_DRIVER,
    ) {
        Ok(p) => p.cast::<PciIo>(),
        Err(s) => return s,
    };
    if remaining_is_end(remaining) {
        // The caller asked for the controller and no child (UEFI 2.x `Start()`).
        return SUCCESS;
    }
    match start_on(d, controller, pci) {
        Ok(()) => SUCCESS,
        Err(s) => {
            debug!("kf-gop: Start failed, status {:#x}", s);
            d.bs.close_protocol(
                controller,
                &crate::efi_unsafe::PCI_IO_GUID,
                image,
                controller,
            );
            s
        }
    }
}

fn refuse(why: Refusal) -> Status {
    debug!("kf-gop: Start refuses: {}", why);
    UNSUPPORTED
}

fn start_on(d: &Driver, controller: Handle, pci: *mut PciIo) -> Result<(), Status> {
    let boot = examine(pci).map_err(refuse)?;
    let saved = attributes(pci, ATTR_GET, 0)?;
    let supports = attributes(pci, ATTR_SUPPORTED, 0)?;
    if supports & ATTR_MEMORY == 0 {
        return Err(refuse(Refusal::NoMemoryDecode));
    }
    attributes(pci, ATTR_ENABLE, ATTR_MEMORY)?;
    let r = bar_range(d.bs, pci, boot.fb.bar)
        .and_then(|(base, len)| probe::place(&boot.fb, base, len))
        .map_err(refuse)
        .and_then(|fb_base| build(d, controller, pci, &boot, fb_base, saved));
    if r.is_err() {
        let _ = attributes(pci, ATTR_SET, saved);
    }
    r
}

/// Size of a device path without its end node.
///
/// # Safety
/// `p` points at a well-formed device path (one the firmware installed).
unsafe fn device_path_bytes(p: *const DevicePath) -> usize {
    let mut n = 0usize;
    let mut q = p.cast::<u8>();
    loop {
        // SAFETY: the caller guarantees a well-formed path: every node header is readable and its
        // length leads to the next node, up to an end node.
        let (ty, len) = unsafe { (*q, usize::from(u16::from_le_bytes([*q.add(2), *q.add(3)]))) };
        if ty == DP_END || len < 4 {
            return n;
        }
        n += len;
        // SAFETY: as above; the next node starts `len` bytes on.
        q = unsafe { q.add(len) };
    }
}

fn build(
    d: &Driver,
    controller: Handle,
    pci: *mut PciIo,
    boot: &Boot,
    fb_base: u64,
    saved: u64,
) -> Result<(), Status> {
    let bs = d.bs;
    let image = d.binding.image_handle;
    let g = boot.fb.geometry;
    let surface = Surface {
        width: g.width as usize,
        height: g.height as usize,
        pitch: g.pitch as usize,
    };
    let shadow_pages = surface.bytes().div_ceil(PAGE);

    let parent = bs.open_protocol(
        controller,
        &DEVICE_PATH_GUID,
        image,
        controller,
        OPEN_GET_PROTOCOL,
    )?;
    // SAFETY: the device-path protocol instance on a PCI controller is a well-formed path.
    let parent_bytes = unsafe { device_path_bytes(parent.cast()) };
    let dp_bytes = parent_bytes + 8 + 4;
    let dp = bs.allocate_pool(dp_bytes)?.cast::<u8>();
    // SAFETY: `dp` holds `parent_bytes + 12` bytes; the parent path is readable for `parent_bytes`
    // (just walked); the ACPI `_ADR` node (8 bytes) and the end node (4 bytes) fill the rest.
    unsafe {
        core::ptr::copy_nonoverlapping(parent.cast::<u8>(), dp, parent_bytes);
        let adr = dp.add(parent_bytes);
        adr.copy_from_nonoverlapping([DP_ACPI, DP_ACPI_ADR, 8, 0].as_ptr(), 4);
        adr.add(4)
            .copy_from_nonoverlapping(ACPI_ADR_VGA.to_le_bytes().as_ptr(), 4);
        adr.add(8).copy_from_nonoverlapping(DP_END_NODE.as_ptr(), 4);
    }

    let shadow = match bs.allocate_pages(shadow_pages) {
        Ok(a) => a,
        Err(s) => {
            // SAFETY: `dp` is our pool allocation, referenced by nothing yet.
            unsafe { bs.free_pool(dp.cast()) };
            return Err(s);
        }
    };
    let ctx = match bs.allocate_pool(size_of::<Ctx>()) {
        Ok(p) => p.cast::<Ctx>(),
        Err(s) => {
            // SAFETY: both are ours and referenced by nothing yet.
            unsafe {
                bs.free_pages(shadow, shadow_pages);
                bs.free_pool(dp.cast());
            }
            return Err(s);
        }
    };
    let mut edid = [0u8; EDID_MAX];
    edid[..boot.edid_len].copy_from_slice(boot.edid());
    // SAFETY: `ctx` is a fresh pool allocation of `size_of::<Ctx>()` bytes, 8-byte aligned like `Ctx`.
    // The inner pointers are set right after, from the allocation's own address, which never moves.
    unsafe {
        ctx.write(Ctx {
            gop: Gop {
                query_mode,
                set_mode,
                blt,
                mode: core::ptr::null_mut(),
            },
            signature: CTX_SIGNATURE,
            bs,
            image,
            controller,
            child: core::ptr::null_mut(),
            pci,
            saved_attrs: saved,
            mode: GopMode {
                max_mode: 1,
                mode: 0,
                info: core::ptr::null_mut(),
                size_of_info: size_of::<ModeInfo>(),
                frame_buffer_base: fb_base,
                frame_buffer_size: g.fb_size as usize,
            },
            info: ModeInfo {
                version: 0,
                horizontal_resolution: g.width,
                vertical_resolution: g.height,
                pixel_format: kf_gop::PIXEL_BGRX,
                pixel_information: [0; 4],
                pixels_per_scan_line: g.pitch / 4,
            },
            edid_discovered: Edid {
                size_of_edid: boot.edid_len as u32,
                edid: core::ptr::null_mut(),
            },
            edid_active: Edid {
                size_of_edid: boot.edid_len as u32,
                edid: core::ptr::null_mut(),
            },
            edid,
            device_path: dp.cast(),
            device_path_bytes: dp_bytes,
            shadow,
            shadow_pages,
            fb_base,
            surface,
        });
        let c = &mut *ctx;
        c.gop.mode = &raw mut c.mode;
        c.mode.info = &raw mut c.info;
        c.edid_discovered.edid = c.edid.as_mut_ptr();
        c.edid_active.edid = c.edid.as_mut_ptr();
    }
    // SAFETY: `ctx` was initialised above and is exclusively ours until its protocols are installed.
    let c = unsafe { &mut *ctx };
    let installed = install_child(c);
    if let Err(s) = installed {
        teardown(c);
        return Err(s);
    }
    debug!(
        "kf-gop: started {}x{} pitch {} fb {:#x} size {:#x} bar {}",
        g.width, g.height, g.pitch, fb_base, g.fb_size, boot.fb.bar
    );
    let _ = clear(c);
    Ok(())
}

fn install_child(c: &mut Ctx) -> Result<(), Status> {
    let bs = c.bs;
    let mut child: Handle = core::ptr::null_mut();
    // SAFETY: every interface below lives inside the context, which is freed only after the
    // interface is uninstalled (`teardown`), and has its protocol's layout.
    unsafe {
        for (guid, iface) in [
            (&DEVICE_PATH_GUID, c.device_path.cast::<c_void>()),
            (&GOP_GUID, (&raw mut c.gop).cast()),
            (&EDID_DISCOVERED_GUID, (&raw mut c.edid_discovered).cast()),
            (&EDID_ACTIVE_GUID, (&raw mut c.edid_active).cast()),
        ] {
            let s = bs.install(&mut child, guid, iface);
            c.child = child;
            if is_error(s) {
                return Err(s);
            }
        }
    }
    bs.open_protocol(
        c.controller,
        &crate::efi_unsafe::PCI_IO_GUID,
        c.image,
        child,
        OPEN_BY_CHILD_CONTROLLER,
    )?;
    Ok(())
}

/// Uninstall what is installed, restore the attributes and free the context. Uninstalling a
/// protocol that was never installed is a harmless `EFI_NOT_FOUND`.
fn teardown(c: &mut Ctx) {
    let bs = c.bs;
    if !c.child.is_null() {
        bs.close_protocol(
            c.controller,
            &crate::efi_unsafe::PCI_IO_GUID,
            c.image,
            c.child,
        );
        bs.uninstall(c.child, &EDID_ACTIVE_GUID, (&raw mut c.edid_active).cast());
        bs.uninstall(
            c.child,
            &EDID_DISCOVERED_GUID,
            (&raw mut c.edid_discovered).cast(),
        );
        bs.uninstall(c.child, &GOP_GUID, (&raw mut c.gop).cast());
        bs.uninstall(c.child, &DEVICE_PATH_GUID, c.device_path.cast());
    }
    let _ = attributes(c.pci, ATTR_SET, c.saved_attrs);
    c.signature = 0;
    let (shadow, pages, dp) = (c.shadow, c.shadow_pages, c.device_path);
    let ctx: *mut Ctx = c;
    // SAFETY: the shadow pages, the device path and the context are this driver's allocations, and
    // nothing references them once the protocols above are uninstalled; `c` is not used after this.
    unsafe {
        bs.free_pages(shadow, pages);
        bs.free_pool(dp.cast());
        bs.free_pool(ctx.cast());
    }
}

extern "efiapi" fn stop(
    this: *mut DriverBinding,
    controller: Handle,
    children: usize,
    child: *mut Handle,
) -> Status {
    let d = driver(this);
    let image = d.binding.image_handle;
    if children == 0 {
        d.bs.close_protocol(
            controller,
            &crate::efi_unsafe::PCI_IO_GUID,
            image,
            controller,
        );
        return SUCCESS;
    }
    for i in 0..children {
        // SAFETY: `Stop()`'s contract: `ChildHandleBuffer` holds `NumberOfChildren` handles.
        let h = unsafe { *child.add(i) };
        let Ok(gop) =
            d.bs.open_protocol(h, &GOP_GUID, image, controller, OPEN_GET_PROTOCOL)
        else {
            continue;
        };
        let Some(c) = ctx_of(gop.cast()) else {
            continue;
        };
        teardown(c);
    }
    SUCCESS
}

extern "efiapi" fn get_driver_name(
    _this: *mut ComponentName2,
    language: *const u8,
    name: *mut *const u16,
) -> Status {
    if language.is_null() || name.is_null() {
        return INVALID_PARAMETER;
    }
    // SAFETY: `Language` is a NUL-terminated ASCII string (UEFI 2.x `GetDriverName()`); at most three
    // bytes are read, stopping at the NUL. `name` is the caller's out-pointer.
    unsafe {
        let en =
            *language == b'e' && *language.add(1) == b'n' && matches!(*language.add(2), 0 | b'-');
        if !en {
            return UNSUPPORTED;
        }
        *name = DRIVER_NAME.as_ptr();
    }
    SUCCESS
}

extern "efiapi" fn get_controller_name(
    _this: *mut ComponentName2,
    _controller: Handle,
    _child: Handle,
    _language: *const u8,
    _name: *mut *const u16,
) -> Status {
    UNSUPPORTED
}

/// The context behind a GOP pointer, if it is one of ours.
fn ctx_of<'a>(gop: *mut Gop) -> Option<&'a mut Ctx> {
    if gop.is_null() {
        return None;
    }
    let ctx = gop.cast::<Ctx>();
    // SAFETY: GOP functions are called with the interface pointer installed on our child, which is the
    // first field of a live `#[repr(C)]` `Ctx`; the signature check rejects anything else that reached
    // here by the same table (a caller passing a foreign GOP to our function).
    unsafe { ((*ctx).signature == CTX_SIGNATURE).then(|| &mut *ctx) }
}

/// ★ The framebuffer, written the only way `blt` may write it: one aligned, volatile 32-bit store per
/// pixel (`docs/OWNER_RULINGS.md` §K — a BAR mapped as Device memory on arm64 faults on an unaligned
/// access and does not permit `DC ZVA`; `blt`'s module docs). No reference to the BAR's memory is
/// ever formed, so the compiler cannot turn a copy into `memcpy`/`memset` or read it back.
///
/// Invariant, established where the one value is built ([`Ctx::planes`], the only constructor —
/// the fields are private to this file): `base` is 4-byte aligned (`probe::place`) and
/// `[base, base + 4·words)` lies inside the BAR the PCI bus driver assigned, with memory decode on.
struct Mmio {
    base: *mut u32,
    words: usize,
}

impl Framebuffer for Mmio {
    fn words(&self) -> usize {
        self.words
    }

    fn store(&mut self, i: usize, px: [u8; BPP]) -> Result<(), Invalid> {
        if i >= self.words {
            return Err(Invalid);
        }
        // SAFETY: `i < words`, so `base + 4·i` is a 4-byte-aligned word inside the BAR range the
        // invariant above names (the PCI bus driver's own assignment, decode enabled, identity-mapped
        // boot-services memory). A volatile store of a `u32` is one aligned 32-bit write; nothing
        // else in this driver aliases the BAR, and GOP calls are serialized at `TPL_NOTIFY`.
        unsafe { self.base.add(i).write_volatile(u32::from_ne_bytes(px)) };
        Ok(())
    }
}

impl Ctx {
    /// The shadow (`surface.bytes()` long) and the framebuffer.
    fn planes(&mut self) -> (&mut [u8], Mmio) {
        let n = self.surface.bytes();
        let fb = Mmio {
            base: self.fb_base as *mut u32,
            // `fb_base .. fb_base + G` is inside the BAR (`probe::place`) and `n ≤ G`
            // (`Geometry::check`); `blt` refuses a surface whose pitch is not a word multiple.
            words: n / BPP,
        };
        // SAFETY: `shadow` is this context's `AllocatePages` range of `shadow_pages · 4096 ≥ n` bytes,
        // written only here. It is used only under `TPL_NOTIFY`, so no second borrow exists while
        // this one lives.
        let shadow = unsafe { core::slice::from_raw_parts_mut(self.shadow as *mut u8, n) };
        (shadow, fb)
    }
}

fn clear(c: &mut Ctx) -> Status {
    let old = c.bs.raise_tpl(TPL_NOTIFY);
    let surface = c.surface;
    let (shadow, mut fb) = c.planes();
    let r = blt::clear(&surface, shadow, &mut fb);
    c.bs.restore_tpl(old);
    if r.is_ok() {
        SUCCESS
    } else {
        crate::efi_unsafe::DEVICE_ERROR
    }
}

extern "efiapi" fn query_mode(
    this: *mut Gop,
    mode: u32,
    size: *mut usize,
    info: *mut *mut ModeInfo,
) -> Status {
    let Some(c) = ctx_of(this) else {
        return INVALID_PARAMETER;
    };
    if mode != 0 || size.is_null() || info.is_null() {
        return INVALID_PARAMETER;
    }
    let Ok(p) = c.bs.allocate_pool(size_of::<ModeInfo>()) else {
        return OUT_OF_RESOURCES;
    };
    // SAFETY: `p` is a fresh pool allocation of one `ModeInfo` (alignment 4 ≤ 8); `size` and `info`
    // are the caller's non-null out-pointers. The caller frees `*info` (UEFI 2.x `QueryMode()`).
    unsafe {
        p.cast::<ModeInfo>().write(c.info);
        *size = size_of::<ModeInfo>();
        *info = p.cast();
    }
    SUCCESS
}

extern "efiapi" fn set_mode(this: *mut Gop, mode: u32) -> Status {
    let Some(c) = ctx_of(this) else {
        return INVALID_PARAMETER;
    };
    if mode != 0 {
        return UNSUPPORTED;
    }
    clear(c)
}

#[allow(clippy::too_many_arguments)]
extern "efiapi" fn blt(
    this: *mut Gop,
    buffer: *mut u8,
    op: u32,
    sx: usize,
    sy: usize,
    dx: usize,
    dy: usize,
    w: usize,
    h: usize,
    delta: usize,
) -> Status {
    let Some(c) = ctx_of(this) else {
        return INVALID_PARAMETER;
    };
    let Some(op) = blt::Op::from_raw(op) else {
        return INVALID_PARAMETER;
    };
    let Ok(plan) = blt::plan(
        &blt::Request {
            op,
            sx,
            sy,
            dx,
            dy,
            w,
            h,
            delta,
        },
        &c.surface,
    ) else {
        return INVALID_PARAMETER;
    };
    let n = plan.buffer_bytes();
    if n > 0 && buffer.is_null() {
        return INVALID_PARAMETER;
    }
    // SAFETY: the caller's `BltBuffer` covers the rectangle the operation names (UEFI 2.x `Blt()`), and
    // `n` is exactly the bytes from its start that the rectangle spans (`Plan::buffer_bytes`). It is
    // borrowed only for this call.
    let buf = unsafe {
        match plan.access() {
            Access::None => Buffer::None,
            Access::Read => Buffer::Read(core::slice::from_raw_parts(buffer, n)),
            Access::Write => Buffer::Write(core::slice::from_raw_parts_mut(buffer, n)),
        }
    };
    let old = c.bs.raise_tpl(TPL_NOTIFY);
    let (shadow, mut fb) = c.planes();
    let r = blt::execute(&plan, shadow, &mut fb, buf);
    c.bs.restore_tpl(old);
    if r.is_ok() {
        SUCCESS
    } else {
        INVALID_PARAMETER
    }
}
