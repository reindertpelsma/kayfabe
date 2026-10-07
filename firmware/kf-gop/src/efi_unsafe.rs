// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! The UEFI ABI this firmware needs, and nothing more: the system and boot-services tables, the
//! protocols it consumes (PCI I/O, device path) and produces (driver binding, component name, GOP,
//! EDID), and thin wrappers around the boot services it calls.
//!
//! Layouts follow the UEFI 2.x specification and were read field by field against EDK2's headers
//! (`MdePkg/Include/Uefi/UefiSpec.h`, `Protocol/PciIo.h`, `Protocol/GraphicsOutput.h`,
//! `Protocol/DriverBinding.h`, `Protocol/ComponentName2.h`, `Protocol/EdidDiscovered.h`,
//! `Protocol/EdidActive.h`, `Protocol/DevicePath.h`; edk2-stable202408 in QEMU 10.2.4's
//! `roms/edk2`). A table entry this firmware never calls is a `usize` placeholder of the same size.
//!
//! Shared by both binaries through `#[path]`; each uses a subset, hence the `dead_code` allowance.
#![allow(dead_code)]

use core::ffi::c_void;

/// `EFI_HANDLE`.
pub type Handle = *mut c_void;
/// `EFI_STATUS`.
pub type Status = usize;

const ERR: usize = 1 << (usize::BITS - 1);
/// `EFI_SUCCESS`.
pub const SUCCESS: Status = 0;
/// `EFI_INVALID_PARAMETER`.
pub const INVALID_PARAMETER: Status = ERR | 2;
/// `EFI_UNSUPPORTED`.
pub const UNSUPPORTED: Status = ERR | 3;
/// `EFI_DEVICE_ERROR`.
pub const DEVICE_ERROR: Status = ERR | 7;
/// `EFI_OUT_OF_RESOURCES`.
pub const OUT_OF_RESOURCES: Status = ERR | 9;
/// `EFI_NOT_FOUND`.
pub const NOT_FOUND: Status = ERR | 14;

/// Whether a status is an error.
pub fn is_error(s: Status) -> bool {
    s & ERR != 0
}

/// `EFI_GUID`.
#[repr(C)]
pub struct Guid(pub u32, pub u16, pub u16, pub [u8; 8]);

/// `EFI_PCI_IO_PROTOCOL_GUID`.
pub static PCI_IO_GUID: Guid = Guid(
    0x4cf5_b200,
    0x68b8,
    0x4ca5,
    [0x9e, 0xec, 0xb2, 0x3e, 0x3f, 0x50, 0x02, 0x9a],
);
/// `EFI_DEVICE_PATH_PROTOCOL_GUID`.
pub static DEVICE_PATH_GUID: Guid = Guid(
    0x0957_6e91,
    0x6d3f,
    0x11d2,
    [0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
);
/// `EFI_DRIVER_BINDING_PROTOCOL_GUID`.
pub static DRIVER_BINDING_GUID: Guid = Guid(
    0x18a0_31ab,
    0xb443,
    0x4d1a,
    [0xa5, 0xc0, 0x0c, 0x09, 0x26, 0x1e, 0x9f, 0x71],
);
/// `EFI_COMPONENT_NAME2_PROTOCOL_GUID`.
pub static COMPONENT_NAME2_GUID: Guid = Guid(
    0x6a7a_5cff,
    0xe8d9,
    0x4f70,
    [0xba, 0xda, 0x75, 0xab, 0x30, 0x25, 0xce, 0x14],
);
/// `EFI_GRAPHICS_OUTPUT_PROTOCOL_GUID`.
pub static GOP_GUID: Guid = Guid(
    0x9042_a9de,
    0x23dc,
    0x4a38,
    [0x96, 0xfb, 0x7a, 0xde, 0xd0, 0x80, 0x51, 0x6a],
);
/// `EFI_EDID_DISCOVERED_PROTOCOL_GUID`.
pub static EDID_DISCOVERED_GUID: Guid = Guid(
    0x1c0c_34f6,
    0xd380,
    0x41fa,
    [0xa0, 0x49, 0x8a, 0xd0, 0x6c, 0x1a, 0x66, 0xaa],
);
/// `EFI_EDID_ACTIVE_PROTOCOL_GUID`.
pub static EDID_ACTIVE_GUID: Guid = Guid(
    0xbd8c_1056,
    0x9f36,
    0x44ec,
    [0x92, 0xa8, 0xa6, 0x33, 0x7f, 0x81, 0x79, 0x86],
);

/// `EFI_TABLE_HEADER`.
#[repr(C)]
pub struct TableHeader {
    signature: u64,
    revision: u32,
    header_size: u32,
    crc32: u32,
    reserved: u32,
}

/// `EFI_SYSTEM_TABLE`.
#[repr(C)]
pub struct SystemTable {
    hdr: TableHeader,
    firmware_vendor: *const u16,
    firmware_revision: u32,
    console_in_handle: Handle,
    con_in: *mut c_void,
    console_out_handle: Handle,
    con_out: *mut c_void,
    standard_error_handle: Handle,
    std_err: *mut c_void,
    runtime_services: *mut RuntimeServices,
    boot_services: *mut BootServices,
    number_of_table_entries: usize,
    configuration_table: *mut c_void,
}

/// `EFI_GLOBAL_VARIABLE`: the vendor GUID of `SecureBoot` and the other architectural variables.
pub static GLOBAL_VARIABLE_GUID: Guid = Guid(
    0x8be4_df61,
    0x93ca,
    0x11d2,
    [0xaa, 0x0d, 0x00, 0xe0, 0x98, 0x03, 0x2b, 0x8c],
);

/// `EFI_RUNTIME_SERVICES`, up to `GetVariable` (the only entry used).
#[repr(C)]
pub struct RuntimeServices {
    hdr: TableHeader,
    get_time: usize,
    set_time: usize,
    get_wakeup_time: usize,
    set_wakeup_time: usize,
    set_virtual_address_map: usize,
    convert_pointer: usize,
    get_variable: unsafe extern "efiapi" fn(
        name: *const u16,
        vendor: *const Guid,
        attributes: *mut u32,
        size: *mut usize,
        data: *mut c_void,
    ) -> Status,
}

/// The `SecureBoot` variable: 1 when the firmware enforces image verification, 0 when not; the
/// `GetVariable` status when it cannot be read (`EFI_NOT_FOUND` on a firmware with no keys at all).
///
/// # Safety
/// `st` is the `EFI_SYSTEM_TABLE` pointer the firmware passed to the entry point.
pub unsafe fn secure_boot(st: *mut SystemTable) -> Result<u8, Status> {
    const NAME: [u16; 11] = [
        b'S' as u16,
        b'e' as u16,
        b'c' as u16,
        b'u' as u16,
        b'r' as u16,
        b'e' as u16,
        b'B' as u16,
        b'o' as u16,
        b'o' as u16,
        b't' as u16,
        0,
    ];
    let mut v = 0u8;
    let mut n = 1usize;
    // SAFETY: the caller passes the firmware's system table, whose runtime-services table is valid
    // during boot services; `NAME` is NUL-terminated; `v` is a one-byte local and `n` says so.
    let s = unsafe {
        let rt = (*st).runtime_services;
        ((*rt).get_variable)(
            NAME.as_ptr(),
            &GLOBAL_VARIABLE_GUID,
            core::ptr::null_mut(),
            &mut n,
            (&raw mut v).cast(),
        )
    };
    if is_error(s) { Err(s) } else { Ok(v) }
}

/// `EFI_TPL` for `TPL_NOTIFY`: GOP calls run here so a timer callback cannot re-enter a `Blt`.
pub const TPL_NOTIFY: usize = 16;
/// `AllocateAnyPages`.
const ALLOCATE_ANY_PAGES: u32 = 0;
/// `EfiBootServicesData`: everything this driver allocates is gone after `ExitBootServices`, and so
/// is the GOP.
const BOOT_SERVICES_DATA: u32 = 4;
/// `EFI_NATIVE_INTERFACE`.
const NATIVE_INTERFACE: u32 = 0;
/// `ByProtocol` for `LocateHandleBuffer`.
const BY_PROTOCOL: u32 = 2;

/// `OpenProtocol` attribute `EFI_OPEN_PROTOCOL_GET_PROTOCOL`.
pub const OPEN_GET_PROTOCOL: u32 = 0x02;
/// `EFI_OPEN_PROTOCOL_BY_CHILD_CONTROLLER`.
pub const OPEN_BY_CHILD_CONTROLLER: u32 = 0x08;
/// `EFI_OPEN_PROTOCOL_BY_DRIVER`.
pub const OPEN_BY_DRIVER: u32 = 0x10;

/// `EFI_BOOT_SERVICES`, in table order.
#[repr(C)]
pub struct BootServices {
    hdr: TableHeader,
    raise_tpl: unsafe extern "efiapi" fn(new: usize) -> usize,
    restore_tpl: unsafe extern "efiapi" fn(old: usize),
    allocate_pages:
        unsafe extern "efiapi" fn(ty: u32, mem: u32, pages: usize, addr: *mut u64) -> Status,
    free_pages: unsafe extern "efiapi" fn(addr: u64, pages: usize) -> Status,
    get_memory_map: usize,
    allocate_pool:
        unsafe extern "efiapi" fn(mem: u32, size: usize, out: *mut *mut c_void) -> Status,
    free_pool: unsafe extern "efiapi" fn(buf: *mut c_void) -> Status,
    create_event: usize,
    set_timer: usize,
    wait_for_event: usize,
    signal_event: usize,
    close_event: usize,
    check_event: usize,
    install_protocol_interface: unsafe extern "efiapi" fn(
        handle: *mut Handle,
        guid: *const Guid,
        ty: u32,
        iface: *mut c_void,
    ) -> Status,
    reinstall_protocol_interface: usize,
    uninstall_protocol_interface:
        unsafe extern "efiapi" fn(handle: Handle, guid: *const Guid, iface: *mut c_void) -> Status,
    handle_protocol: unsafe extern "efiapi" fn(
        handle: Handle,
        guid: *const Guid,
        out: *mut *mut c_void,
    ) -> Status,
    reserved: usize,
    register_protocol_notify: usize,
    locate_handle: usize,
    locate_device_path: unsafe extern "efiapi" fn(
        guid: *const Guid,
        path: *mut *mut DevicePath,
        device: *mut Handle,
    ) -> Status,
    install_configuration_table: usize,
    load_image: usize,
    start_image: usize,
    exit: usize,
    unload_image: usize,
    exit_boot_services: usize,
    get_next_monotonic_count: usize,
    stall: unsafe extern "efiapi" fn(us: usize) -> Status,
    set_watchdog_timer: unsafe extern "efiapi" fn(
        timeout: usize,
        code: u64,
        size: usize,
        data: *const u16,
    ) -> Status,
    connect_controller: usize,
    disconnect_controller: usize,
    open_protocol: unsafe extern "efiapi" fn(
        handle: Handle,
        guid: *const Guid,
        out: *mut *mut c_void,
        agent: Handle,
        controller: Handle,
        attrs: u32,
    ) -> Status,
    close_protocol: unsafe extern "efiapi" fn(
        handle: Handle,
        guid: *const Guid,
        agent: Handle,
        controller: Handle,
    ) -> Status,
    open_protocol_information: usize,
    protocols_per_handle: usize,
    locate_handle_buffer: unsafe extern "efiapi" fn(
        ty: u32,
        guid: *const Guid,
        key: *mut c_void,
        count: *mut usize,
        out: *mut *mut Handle,
    ) -> Status,
    locate_protocol: usize,
    install_multiple_protocol_interfaces: usize,
    uninstall_multiple_protocol_interfaces: usize,
    calculate_crc32: usize,
    copy_mem: usize,
    set_mem: usize,
    create_event_ex: usize,
}

/// `EFI_DEVICE_PATH_PROTOCOL`: one node header.
#[repr(C)]
pub struct DevicePath {
    /// Node type.
    pub ty: u8,
    /// Node sub-type.
    pub sub_type: u8,
    /// Node length, little-endian, header included.
    pub length: [u8; 2],
}

/// Device-path node type: end of path.
pub const DP_END: u8 = 0x7F;
/// Device-path sub-type: end of the entire path.
pub const DP_END_ENTIRE: u8 = 0xFF;
/// Device-path node type: ACPI.
pub const DP_ACPI: u8 = 0x02;
/// ACPI sub-type: `_ADR`.
pub const DP_ACPI_ADR: u8 = 0x03;
/// The end-of-entire-path node, as bytes.
pub const DP_END_NODE: [u8; 4] = [DP_END, DP_END_ENTIRE, 4, 0];

/// `EFI_PCI_IO_PROTOCOL_ACCESS` for `Mem` and `Io`.
#[repr(C)]
pub struct PciIoBarAccess {
    read: usize,
    write: usize,
}

/// `EFI_PCI_IO_PROTOCOL_CONFIG_ACCESS`.
#[repr(C)]
pub struct PciIoConfigAccess {
    /// `Pci.Read`.
    pub read: unsafe extern "efiapi" fn(
        this: *mut PciIo,
        width: u32,
        offset: u32,
        count: usize,
        buffer: *mut c_void,
    ) -> Status,
    write: usize,
}

/// `EfiPciIoAttributeOperationGet`.
pub const ATTR_GET: u32 = 0;
/// `EfiPciIoAttributeOperationSet`.
pub const ATTR_SET: u32 = 1;
/// `EfiPciIoAttributeOperationEnable`.
pub const ATTR_ENABLE: u32 = 2;
/// `EfiPciIoAttributeOperationSupported`.
pub const ATTR_SUPPORTED: u32 = 4;
/// `EFI_PCI_IO_ATTRIBUTE_MEMORY`: memory decode, and nothing else. kf3 has no I/O BAR and the GOP
/// does no DMA, so neither I/O decode nor bus mastering is asked for.
pub const ATTR_MEMORY: u64 = 0x0200;

/// `EFI_PCI_IO_PROTOCOL`.
#[repr(C)]
pub struct PciIo {
    poll_mem: usize,
    poll_io: usize,
    mem: PciIoBarAccess,
    io: PciIoBarAccess,
    /// Config-space access.
    pub pci: PciIoConfigAccess,
    copy_mem: usize,
    map: usize,
    unmap: usize,
    allocate_buffer: usize,
    free_buffer: usize,
    flush: usize,
    get_location: usize,
    /// `Attributes()`.
    pub attributes: unsafe extern "efiapi" fn(
        this: *mut PciIo,
        op: u32,
        attrs: u64,
        result: *mut u64,
    ) -> Status,
    /// `GetBarAttributes()`.
    pub get_bar_attributes: unsafe extern "efiapi" fn(
        this: *mut PciIo,
        bar: u8,
        supports: *mut u64,
        resources: *mut *mut c_void,
    ) -> Status,
    set_bar_attributes: usize,
    /// Bytes at `rom_image`.
    pub rom_size: u64,
    /// The PCI bus driver's in-memory copy of the option ROM.
    pub rom_image: *mut c_void,
}

/// `EFI_DRIVER_BINDING_PROTOCOL`.
#[repr(C)]
pub struct DriverBinding {
    /// `Supported()`.
    pub supported: unsafe extern "efiapi" fn(
        this: *mut DriverBinding,
        controller: Handle,
        remaining: *mut DevicePath,
    ) -> Status,
    /// `Start()`.
    pub start: unsafe extern "efiapi" fn(
        this: *mut DriverBinding,
        controller: Handle,
        remaining: *mut DevicePath,
    ) -> Status,
    /// `Stop()`.
    pub stop: unsafe extern "efiapi" fn(
        this: *mut DriverBinding,
        controller: Handle,
        children: usize,
        child: *mut Handle,
    ) -> Status,
    /// Version; higher wins among general drivers. A ROM driver is tried first anyway, through the
    /// PCI bus driver's bus-specific override.
    pub version: u32,
    /// The image that produced it.
    pub image_handle: Handle,
    /// The handle it is installed on.
    pub driver_binding_handle: Handle,
}

/// `EFI_COMPONENT_NAME2_PROTOCOL`.
#[repr(C)]
pub struct ComponentName2 {
    /// `GetDriverName()`.
    pub get_driver_name: unsafe extern "efiapi" fn(
        this: *mut ComponentName2,
        language: *const u8,
        name: *mut *const u16,
    ) -> Status,
    /// `GetControllerName()`.
    pub get_controller_name: unsafe extern "efiapi" fn(
        this: *mut ComponentName2,
        controller: Handle,
        child: Handle,
        language: *const u8,
        name: *mut *const u16,
    ) -> Status,
    /// RFC 4646 language codes, `;`-separated, NUL-terminated.
    pub supported_languages: *const u8,
}

/// `EFI_GRAPHICS_OUTPUT_MODE_INFORMATION`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ModeInfo {
    /// Structure version, 0.
    pub version: u32,
    /// Width.
    pub horizontal_resolution: u32,
    /// Height.
    pub vertical_resolution: u32,
    /// `EFI_GRAPHICS_PIXEL_FORMAT`.
    pub pixel_format: u32,
    /// Masks for `PixelBitMask` only; zero here.
    pub pixel_information: [u32; 4],
    /// Pixels from one line to the next.
    pub pixels_per_scan_line: u32,
}

/// `EFI_GRAPHICS_OUTPUT_PROTOCOL_MODE`.
#[repr(C)]
pub struct GopMode {
    /// Modes offered.
    pub max_mode: u32,
    /// The current mode.
    pub mode: u32,
    /// The current mode's information.
    pub info: *mut ModeInfo,
    /// `sizeof(*info)`.
    pub size_of_info: usize,
    /// Guest-physical framebuffer base.
    pub frame_buffer_base: u64,
    /// Framebuffer bytes.
    pub frame_buffer_size: usize,
}

/// `EFI_GRAPHICS_OUTPUT_BLT_PIXEL` is four bytes (B, G, R, reserved); the driver only ever sees it as
/// bytes, so the type is opaque here.
pub type BltPixel = u8;

/// `EFI_GRAPHICS_OUTPUT_PROTOCOL`.
#[repr(C)]
pub struct Gop {
    /// `QueryMode()`.
    pub query_mode: unsafe extern "efiapi" fn(
        this: *mut Gop,
        mode: u32,
        size: *mut usize,
        info: *mut *mut ModeInfo,
    ) -> Status,
    /// `SetMode()`.
    pub set_mode: unsafe extern "efiapi" fn(this: *mut Gop, mode: u32) -> Status,
    /// `Blt()`.
    #[allow(clippy::type_complexity)]
    pub blt: unsafe extern "efiapi" fn(
        this: *mut Gop,
        buffer: *mut BltPixel,
        op: u32,
        sx: usize,
        sy: usize,
        dx: usize,
        dy: usize,
        w: usize,
        h: usize,
        delta: usize,
    ) -> Status,
    /// The mode block.
    pub mode: *mut GopMode,
}

/// `EFI_EDID_DISCOVERED_PROTOCOL` and `EFI_EDID_ACTIVE_PROTOCOL` (same layout).
#[repr(C)]
pub struct Edid {
    /// Bytes at `edid`.
    pub size_of_edid: u32,
    /// The EDID.
    pub edid: *mut u8,
}

/// The boot-services table, valid from the entry point until `ExitBootServices`. Every GOP and
/// driver-binding entry runs inside that window: a GOP is a boot-time protocol and its memory is
/// `EfiBootServicesData`.
#[derive(Clone, Copy)]
pub struct Bs(*mut BootServices);

impl Bs {
    /// The boot services of the system table the image entry point received.
    ///
    /// # Safety
    /// `st` is the `EFI_SYSTEM_TABLE` pointer the firmware passed to the entry point.
    pub unsafe fn from_system_table(st: *mut SystemTable) -> Bs {
        // SAFETY: the caller passes the firmware's system table, which is valid and whose
        // `BootServices` field points at the boot-services table for the whole boot-services phase.
        Bs(unsafe { (*st).boot_services })
    }

    /// `RaiseTPL(new)`; returns the previous level for [`Bs::restore_tpl`].
    pub fn raise_tpl(self, new: usize) -> usize {
        // SAFETY: `self.0` is the live boot-services table (type invariant); RaiseTPL takes a value.
        unsafe { ((*self.0).raise_tpl)(new) }
    }

    /// `RestoreTPL(old)`.
    pub fn restore_tpl(self, old: usize) {
        // SAFETY: as in `raise_tpl`; `old` came from the matching `raise_tpl`.
        unsafe { ((*self.0).restore_tpl)(old) }
    }

    /// `AllocatePool(EfiBootServicesData, size)`.
    pub fn allocate_pool(self, size: usize) -> Result<*mut c_void, Status> {
        let mut p = core::ptr::null_mut();
        // SAFETY: live table (type invariant); `p` is a local the call writes once.
        let s = unsafe { ((*self.0).allocate_pool)(BOOT_SERVICES_DATA, size, &mut p) };
        if is_error(s) || p.is_null() {
            Err(if is_error(s) { s } else { OUT_OF_RESOURCES })
        } else {
            Ok(p)
        }
    }

    /// `FreePool(p)`.
    ///
    /// # Safety
    /// `p` came from `AllocatePool` (this driver's or the firmware's, as the protocol that returned it
    /// says) and is not used afterwards.
    pub unsafe fn free_pool(self, p: *mut c_void) {
        // SAFETY: live table; the caller guarantees `p` is a pool allocation no one uses afterwards.
        unsafe { ((*self.0).free_pool)(p) };
    }

    /// `AllocatePages(AllocateAnyPages, EfiBootServicesData, pages)`: a physical address.
    pub fn allocate_pages(self, pages: usize) -> Result<u64, Status> {
        let mut addr = 0u64;
        // SAFETY: live table; `addr` is a local the call writes once.
        let s = unsafe {
            ((*self.0).allocate_pages)(ALLOCATE_ANY_PAGES, BOOT_SERVICES_DATA, pages, &mut addr)
        };
        if is_error(s) { Err(s) } else { Ok(addr) }
    }

    /// `FreePages(addr, pages)`.
    ///
    /// # Safety
    /// `addr`/`pages` are exactly one earlier [`Bs::allocate_pages`] result, no longer referenced.
    pub unsafe fn free_pages(self, addr: u64, pages: usize) {
        // SAFETY: live table; the caller guarantees the range is ours and unused.
        unsafe { ((*self.0).free_pages)(addr, pages) };
    }

    /// `OpenProtocol`. Handles are validated by the firmware's handle database; `out` is a local.
    pub fn open_protocol(
        self,
        handle: Handle,
        guid: &'static Guid,
        agent: Handle,
        controller: Handle,
        attrs: u32,
    ) -> Result<*mut c_void, Status> {
        let mut out = core::ptr::null_mut();
        // SAFETY: live table; `guid` is a static; `out` is a local; handles are checked by the firmware.
        let s =
            unsafe { ((*self.0).open_protocol)(handle, guid, &mut out, agent, controller, attrs) };
        if is_error(s) { Err(s) } else { Ok(out) }
    }

    /// `CloseProtocol`.
    pub fn close_protocol(
        self,
        handle: Handle,
        guid: &'static Guid,
        agent: Handle,
        controller: Handle,
    ) -> Status {
        // SAFETY: live table; `guid` is a static; handles are checked by the firmware.
        unsafe { ((*self.0).close_protocol)(handle, guid, agent, controller) }
    }

    /// `HandleProtocol`.
    pub fn handle_protocol(
        self,
        handle: Handle,
        guid: &'static Guid,
    ) -> Result<*mut c_void, Status> {
        let mut out = core::ptr::null_mut();
        // SAFETY: live table; `guid` is a static; `out` is a local; the handle is checked by the firmware.
        let s = unsafe { ((*self.0).handle_protocol)(handle, guid, &mut out) };
        if is_error(s) { Err(s) } else { Ok(out) }
    }

    /// `InstallProtocolInterface(handle, guid, EFI_NATIVE_INTERFACE, iface)`. A null `*handle` makes
    /// a new handle.
    ///
    /// # Safety
    /// `iface` points at a structure of the protocol's layout that stays valid until it is
    /// uninstalled: every consumer will call through it.
    pub unsafe fn install(
        self,
        handle: &mut Handle,
        guid: &'static Guid,
        iface: *mut c_void,
    ) -> Status {
        // SAFETY: live table; `handle` is a live `&mut`; the caller guarantees `iface`.
        unsafe { ((*self.0).install_protocol_interface)(handle, guid, NATIVE_INTERFACE, iface) }
    }

    /// `UninstallProtocolInterface`.
    pub fn uninstall(self, handle: Handle, guid: &'static Guid, iface: *mut c_void) -> Status {
        // SAFETY: live table; the firmware only compares `iface` with what it holds for the handle.
        unsafe { ((*self.0).uninstall_protocol_interface)(handle, guid, iface) }
    }

    /// `LocateHandleBuffer(ByProtocol, guid)`: a pool array the caller frees, and its length.
    pub fn locate_handle_buffer(self, guid: &'static Guid) -> Result<(*mut Handle, usize), Status> {
        let mut n = 0usize;
        let mut buf = core::ptr::null_mut();
        // SAFETY: live table; `guid` is a static; `n` and `buf` are locals.
        let s = unsafe {
            ((*self.0).locate_handle_buffer)(
                BY_PROTOCOL,
                guid,
                core::ptr::null_mut(),
                &mut n,
                &mut buf,
            )
        };
        if is_error(s) { Err(s) } else { Ok((buf, n)) }
    }

    /// `LocateDevicePath(guid, path)`: the handle closest to `path` carrying `guid`.
    ///
    /// # Safety
    /// `path` points at a well-formed device path (a protocol instance the firmware returned).
    pub unsafe fn locate_device_path(
        self,
        guid: &'static Guid,
        path: *mut DevicePath,
    ) -> Result<Handle, Status> {
        let mut p = path;
        let mut h = core::ptr::null_mut();
        // SAFETY: live table; the caller guarantees the path; `p` and `h` are locals.
        let s = unsafe { ((*self.0).locate_device_path)(guid, &mut p, &mut h) };
        if is_error(s) { Err(s) } else { Ok(h) }
    }

    /// `Stall(us)`.
    pub fn stall(self, us: usize) {
        // SAFETY: live table; Stall takes a value.
        unsafe { ((*self.0).stall)(us) };
    }

    /// `SetWatchdogTimer(0, …)`: disable the boot manager's five-minute watchdog.
    pub fn disable_watchdog(self) {
        // SAFETY: live table; a zero timeout disables the timer and reads no data pointer.
        unsafe { ((*self.0).set_watchdog_timer)(0, 0, 0, core::ptr::null()) };
    }
}
