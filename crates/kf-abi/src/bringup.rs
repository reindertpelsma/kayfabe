//! ★ The layouts an RM **client** needs — as opposed to the layouts a **forwarder**
//! decodes.
//!
//! Every other module in this crate exists because the guest sends us bytes and we must
//! understand them: [`crate::generated`] is the decode side, [`crate::view`] is its
//! normalised shape, [`crate::versions`] picks between two wire shapes of one verb. Nothing
//! there ever *originates* traffic.
//!
//! `host_execution_plane.md` §3 changes that. To open an RM connection of our own the host
//! isolate has to **construct** an object tree — a client, a device, a subdevice, a GPU
//! address space, a memory object — and that needs a small set of alloc-parameter structs
//! that the forwarder never had to look at, because the guest always supplied them
//! pre-encoded.
//!
//! That is why this is a module and not four more entries in [`crate::transcribed`]: the
//! entries there are all *defects with a fix* (a version whose tree we have not vendored).
//! These are not defects. They are the alloc-parameter half of the ABI, and they are here
//! permanently.
//!
//! ## Provenance
//!
//! Every struct below is transcribed from the **bench's own driver**,
//! `ogkm-580: 580.159.04`, with the file and line on each field group. The generator does
//! not emit them today only because its slice manifest was written for the decode side;
//! adding them to it is the mechanical follow-up, and until then each carries the same
//! compile-time offset assertions the generated code does — a transcription nothing checks
//! is a rumour.
//!
//! ## The escape numbers here are the ones the FRONTEND owns, not RM
//!
//! [`NV_ESC_REGISTER_FD`] and [`NV_ESC_CHECK_VERSION_STR`] are numbered from
//! `NV_IOCTL_BASE` (200) in `ogkm-580: kernel-open/common/inc/nv-ioctl-numbers.h:5-19`,
//! **not** from the `NV_ESC_RM_*` space in
//! `ogkm-580: src/nvidia/arch/nvalloc/unix/include/nv_escape.h`. Two disjoint numbering
//! schemes share one `_IOWR('F', …)` magic, which is exactly the sort of thing that reads
//! as a typo three months later.

use crate::wire::{AbiError, u32_at, u64_at};

/// The ioctl "magic" every NVIDIA frontend escape uses —
/// `ogkm-580: kernel-open/common/inc/nv-ioctl-numbers.h:4`.
pub const NV_IOCTL_MAGIC: u8 = b'F';

/// `NV_ESC_REGISTER_FD` (`NV_IOCTL_BASE + 1`) —
/// `ogkm-580: kernel-open/common/inc/nv-ioctl-numbers.h:7`.
///
/// ★ **The one binding that is genuinely required**, and the C says why in a comment worth
/// carrying: a per-GPU node used without it answers `0x23 INVALID_CLIENT`, because the RM
/// client lives in the control node's session and the device node has to be joined to it
/// (`C: src/qemu/nvkvm_gpu_emul.c:7217-7231`). Work that touches only the control node does
/// not need it at all.
pub const NV_ESC_REGISTER_FD: u8 = 201;

/// `NV_ESC_CARD_INFO` (`NV_IOCTL_BASE + 0`) —
/// `ogkm-580: kernel-open/common/inc/nv-ioctl-numbers.h:31`.
///
/// ★ Issued on the **control** node (`NV_CTL_DEVICE_ONLY`, `kernel-open/nvidia/nv.c:2527`).
/// It is the frontend's own table of `{minor, PCI address, RM gpuId}` for every probed GPU —
/// the only place the kernel states which **minor** is which **RM GPU**. ⊘ The minor is a
/// Linux character-device number; the RM *device instance* (`NV0080_ALLOC_PARAMETERS
/// .deviceId`) is assigned at attach time, lowest free first. They coincide only by accident
/// (V3_MULTI_GPU_AUDIT §2 blocker 2 — measured: minor 1 was instance 0).
pub const NV_ESC_CARD_INFO: u8 = 200;

/// `nv_ioctl_card_info_t` — `ogkm-580: kernel-open/common/inc/nv-ioctl.h:54-66`, with
/// `nv_pci_info_t` at `:31-38`. Decoded, never encoded: the kernel `memset`s the whole array
/// before filling it (`nv.c:2333`).
///
/// Layout (x86-64 and aarch64 alike; `NV_ALIGN_BYTES(8)` on the 64-bit fields):
/// `valid` +0 (NvBool), `pci_info.domain` +4, `.bus` +8, `.slot` +9, `.function` +10,
/// `.vendor_id` +12, `.device_id` +14, `gpu_id` +16, `interrupt_line` +20, `reg_address` +24,
/// `reg_size` +32, `fb_address` +40, `fb_size` +48, `minor_number` +56, `dev_name[10]` +60,
/// `sizeof` = 72.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CardInfo {
    /// `valid` — the kernel fills entries front to back and leaves the rest zero.
    pub valid: bool,
    /// PCI domain.
    pub domain: u32,
    /// PCI bus.
    pub bus: u8,
    /// PCI slot (device).
    pub slot: u8,
    /// PCI function.
    pub function: u8,
    /// PCI device id.
    pub device_id: u16,
    /// RM's `gpuId` — the key every `NV0000_CTRL_CMD_GPU_*` control takes.
    pub gpu_id: u32,
    /// BAR0 size.
    pub reg_size: u64,
    /// The `/dev/nvidia<minor>` number.
    pub minor: u32,
}

impl CardInfo {
    /// The C typedef name.
    pub const C_NAME: &'static str = "nv_ioctl_card_info_t";
    /// `sizeof`.
    pub const SIZE: usize = 72;
    /// Entries asked for in one call. `nvidia_read_card_info` refuses with `EINVAL` when the
    /// array is shorter than the number of probed GPUs (`nv.c:2337`), so ask for the RM
    /// ceiling (`NV_MAX_DEVICES` = 32) — 2304 bytes, well inside the direct-ioctl size.
    pub const MAX_ENTRIES: usize = 32;

    /// Decode one entry.
    ///
    /// # Errors
    /// [`AbiError::OutOfRange`] on a short buffer.
    pub fn decode(b: &[u8]) -> Result<CardInfo, AbiError> {
        Ok(CardInfo {
            valid: crate::wire::u8_at(b, 0)? != 0,
            domain: u32_at(b, 4)?,
            bus: crate::wire::u8_at(b, 8)?,
            slot: crate::wire::u8_at(b, 9)?,
            function: crate::wire::u8_at(b, 10)?,
            device_id: crate::wire::u16_at(b, 14)?,
            gpu_id: u32_at(b, 16)?,
            reg_size: u64_at(b, 32)?,
            minor: u32_at(b, 56)?,
        })
    }

    /// The PCI address as Linux and CUDA spell it: `dddd:bb:ss.f` (lower-case hex), the
    /// `/sys/bus/pci/devices/<bdf>` name and the form `cuDeviceGetByPCIBusId` accepts.
    #[must_use]
    pub fn bdf(&self) -> String {
        format!("{:04x}:{:02x}:{:02x}.{:x}", self.domain, self.bus, self.slot, self.function)
    }

    /// Decode a whole `CARD_INFO` reply, keeping only valid entries.
    ///
    /// # Errors
    /// As [`CardInfo::decode`].
    pub fn decode_all(b: &[u8]) -> Result<Vec<CardInfo>, AbiError> {
        let mut out = Vec::new();
        for chunk in b.chunks_exact(Self::SIZE) {
            let c = Self::decode(chunk)?;
            if c.valid {
                out.push(c);
            }
        }
        Ok(out)
    }
}

/// `NV0000_CTRL_CMD_GPU_GET_ID_INFO_V2` —
/// `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl0000/ctrl0000gpu.h:172`. On the root client,
/// `NON_PRIVILEGED` (flags `0x109`, `g_client_resource_nvoc.c:813`).
pub const NV0000_CTRL_CMD_GPU_GET_ID_INFO_V2: u32 = 0x205;

/// `NV0000_CTRL_GPU_GET_ID_INFO_V2_PARAMS` — `ctrl0000gpu.h:176-185`. Eight `NvU32`s.
///
/// ★ `deviceInstance` is the value `NV0080_ALLOC_PARAMETERS.deviceId` must carry, and
/// `subDeviceInstance` the value `NV2080_ALLOC_PARAMETERS.subDeviceId` must carry. RM answers
/// only for an **attached** GPU (`gpumgrGetGpuIdInfoV2`), so this is asked after the per-GPU
/// node's open + `REGISTER_FD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GpuIdInfoV2 {
    /// `gpuId` @ +0 — in.
    pub gpu_id: u32,
    /// `deviceInstance` @ +8 — out.
    pub device_instance: u32,
    /// `subDeviceInstance` @ +12 — out.
    pub sub_device_instance: u32,
}

impl GpuIdInfoV2 {
    /// The C typedef name.
    pub const C_NAME: &'static str = "NV0000_CTRL_GPU_GET_ID_INFO_V2_PARAMS";
    /// `sizeof`.
    pub const SIZE: usize = 32;

    /// Encode the request (`gpuId` only; every out field zero).
    ///
    /// # Errors
    /// [`AbiError::Truncated`].
    pub fn encode_request(gpu_id: u32, bytes: &mut [u8]) -> Result<(), AbiError> {
        put(bytes, Self::C_NAME, Self::SIZE, 0, &[0u8; Self::SIZE])?;
        put(bytes, Self::C_NAME, Self::SIZE, 0, &gpu_id.to_le_bytes())
    }

    /// Decode RM's reply.
    ///
    /// # Errors
    /// [`AbiError::OutOfRange`].
    pub fn decode(b: &[u8]) -> Result<GpuIdInfoV2, AbiError> {
        Ok(GpuIdInfoV2 {
            gpu_id: u32_at(b, 0)?,
            device_instance: u32_at(b, 8)?,
            sub_device_instance: u32_at(b, 12)?,
        })
    }
}

/// `NV_ESC_CHECK_VERSION_STR` (`NV_IOCTL_BASE + 10`) —
/// `ogkm-580: kernel-open/common/inc/nv-ioctl-numbers.h:14`.
///
/// **Not a gate on RM operations.** Two independent paths in the C artifact open the
/// control node and immediately allocate a client with no version handshake at all
/// (`C: src/qemu/nvkvm_isolate_handlers.c:690-696`, `C: src/qemu/nvkvm_gpu_emul.c:6411`).
/// Its value is the *version string*, which is what selects an ABI profile.
pub const NV_ESC_CHECK_VERSION_STR: u8 = 210;

/// `NV_ESC_RM_ALLOC_MEMORY` —
/// `ogkm-580: src/nvidia/arch/nvalloc/unix/include/nv_escape.h:27`.
pub const NV_ESC_RM_ALLOC_MEMORY: u8 = 0x27;

/// `NV20_SUBDEVICE_0`.
///
/// ★★ **A re-export, not a second transcription (2026-08-01).** This was a hand-written
/// `0x2080` until the class table grew a generated row for the same constant, at which
/// point the workspace briefly held two descriptions of one number — the exact shape
/// decision #2's quarantine exists to prevent, and one that no test could have caught
/// because both were right. The generated one is authoritative: it is parsed out of
/// `cl2080.h` by `kayfabe-abi-gen`, so it cannot drift from the header, and this alias
/// keeps every existing caller compiling.
pub use crate::generated::classes::NV20_SUBDEVICE_0;

/// `NV01_MEMORY_SYSTEM` — `ogkm-580: src/common/sdk/nvidia/inc/class/cl003e.h`.
pub const NV01_MEMORY_SYSTEM: u32 = 0x003e;

/// ★★★ `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` —
/// `ogkm-580: src/common/sdk/nvidia/inc/class/cl0071.h`.
///
/// **"Here is memory I already have; pin it and let the GPU reach it."** Unlike
/// [`NV01_MEMORY_SYSTEM`], which asks RM to allocate, this class hands RM an address range
/// the caller owns and RM `pin_user_pages`-walks it. It is how `cuMemHostRegister` works,
/// and it is the only route by which memory the **VMM** owns — guest RAM — can become an
/// object the host GPU's MMU can map.
///
/// ## ⚠ Two facts that are not in the name
///
/// - The `pMemory` address must be routed through
///   [`kayfabe_linux_raw::Indirect::describing`]; nothing in this crate may fill the field
///   in, and [`Nvos02ParametersWithFd::p_memory`] says so.
/// - ★ **This class is on [`crate::capability::DENIED_CLASSES`]**, refused by name with
///   [`crate::capability::DeniedBecause::CallerMemoryDescriptor`]. That refusal is about
///   the **guest** asking for it — honouring a guest-issued `OS_DESCRIPTOR` would hand the
///   host driver a guest-chosen pointer. The isolate issuing one over a range **it** owns
///   is the opposite direction and is not what that row denies. The two must not be
///   conflated, and a future edit that "unblocks" the class for the guest because the host
///   path needs it would delete the boundary rather than cross it.
pub const NV01_MEMORY_SYSTEM_OS_DESCRIPTOR: u32 = 0x0071;

/// `NV01_MEMORY_VIRTUAL` — `ogkm-580: src/common/sdk/nvidia/inc/class/cl0070.h:32`.
///
/// ★★ **The object `NV_ESC_RM_MAP_MEMORY_DMA`'s `hDma` field actually names**, and the one
/// the ladder's R9 rung was missing. A `FERMI_VASPACE_A` handle passed there is refused
/// with `NV_ERR_INVALID_OBJECT_HANDLE` (0x33) — measured on hardware, and the C already
/// knew it (`mode2_mapdma_primitive`: *"hDma must be `NV01_MEMORY_VIRTUAL`"*). An address
/// space and a *mappable range within* an address space are two objects, and only the
/// second is a DMA target.
pub const NV01_MEMORY_VIRTUAL: u32 = 0x0070;

/// `NV_MEMORY_VIRTUAL_ALLOCATION_PARAMS` —
/// `ogkm-580: src/common/sdk/nvidia/inc/class/cl0070.h:66-70`.
///
/// Zero `offset` and zero `limit` mean *"the whole of `h_va_space`"* — `limit` is
/// `[IN/OUT]` and RM writes back the range it settled on (`:61-62`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NvMemoryVirtualAllocationParams {
    /// `NvU64 offset` @ +0 — `[IN]`, a floor on the GPU VA `RmMapMemoryDma` may return.
    pub offset: u64,
    /// `NvU64 limit` @ +8 — `[IN/OUT]`, 0 for "the maximum".
    pub limit: u64,
    /// `NvHandle hVASpace` @ +16 — `[IN]`, the `FERMI_VASPACE_A` this range lives in.
    pub h_va_space: u32,
    // +20: four bytes of tail padding to the struct's 8-byte alignment.
}

impl NvMemoryVirtualAllocationParams {
    /// The C typedef name.
    pub const C_NAME: &'static str = "NV_MEMORY_VIRTUAL_ALLOCATION_PARAMS";
    /// `sizeof`.
    pub const SIZE: usize = 24;
    /// `alignof`.
    pub const ALIGN: usize = 8;

    /// Encode into a little-endian image of at least [`Self::SIZE`] bytes.
    ///
    /// # Errors
    /// [`AbiError::Truncated`].
    pub fn encode_into(&self, bytes: &mut [u8]) -> Result<(), AbiError> {
        let n = Self::C_NAME;
        let sz = Self::SIZE;
        put(bytes, n, sz, 0, &self.offset.to_le_bytes())?;
        put(bytes, n, sz, 8, &self.limit.to_le_bytes())?;
        put(bytes, n, sz, 16, &self.h_va_space.to_le_bytes())
    }
}

/// `nv_ioctl_register_fd_t` — `ogkm-580: kernel-open/common/inc/nv-ioctl.h:124-127`.
///
/// One `int`. Carried by [`NV_ESC_REGISTER_FD`], issued **on the per-GPU node** with the
/// control node's descriptor number inside it.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RegisterFd {
    /// `int ctl_fd` @ +0 — a descriptor number in the *issuing process*.
    pub ctl_fd: i32,
}

impl RegisterFd {
    /// The C typedef name.
    pub const C_NAME: &'static str = "nv_ioctl_register_fd_t";
    /// `sizeof`.
    pub const SIZE: usize = 4;

    /// Encode into a little-endian image of at least [`Self::SIZE`] bytes.
    ///
    /// # Errors
    /// [`AbiError::Truncated`].
    pub fn encode_into(&self, bytes: &mut [u8]) -> Result<(), AbiError> {
        put(
            bytes,
            Self::C_NAME,
            Self::SIZE,
            0,
            &self.ctl_fd.to_le_bytes(),
        )
    }
}

/// `NV2080_ALLOC_PARAMETERS` — `ogkm-580: src/common/sdk/nvidia/inc/class/cl2080.h:43-45`.
///
/// One `NvU32`. It is a whole struct because RM discriminates on `paramsSize`, so passing
/// four zero bytes and passing nothing are different requests.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Nv2080AllocParameters {
    /// `NvU32 subDeviceId` @ +0.
    pub sub_device_id: u32,
}

impl Nv2080AllocParameters {
    /// The C typedef name.
    pub const C_NAME: &'static str = "NV2080_ALLOC_PARAMETERS";
    /// `sizeof`.
    pub const SIZE: usize = 4;

    /// Encode into a little-endian image of at least [`Self::SIZE`] bytes.
    ///
    /// # Errors
    /// [`AbiError::Truncated`].
    pub fn encode_into(&self, bytes: &mut [u8]) -> Result<(), AbiError> {
        put(
            bytes,
            Self::C_NAME,
            Self::SIZE,
            0,
            &self.sub_device_id.to_le_bytes(),
        )
    }
}

/// ★★★★ **§16.28 — `NV_VASPACE_ALLOCATION_INDEX_GPU_DEVICE`**, the `index` value that
/// makes a `FERMI_VASPACE_A` alloc **not the creation of an address space**.
///
/// `[src]` `ogkm-580: src/common/sdk/nvidia/inc/nvos.h:3187` —
/// `#define NV_VASPACE_ALLOCATION_INDEX_GPU_DEVICE 0x03 //<! Acquire reference to device
/// vaspace`. Its sibling `NV_VASPACE_ALLOCATION_INDEX_GPU_NEW` is `0x00` (`:3184`,
/// *"Create new VASpace, by default"*), so the two readings are one wire field apart and
/// the default is the one that creates.
///
/// # ★★★ Why this single value decides a whole route, and where the alloc comes from
///
/// A channel that declares neither `hVASpace` nor `hCtxShare` and is parented on a
/// **Device** gets that **Device's default VA space** — RM allocates a wrapper TSG for it
/// (`ogkm-580: kernel_channel.c:350-375`, *"There is no point in mirroring this allocation
/// in the host"*), whose CtxShare resolves `hVASpace == NV01_NULL_OBJECT` through
/// `vaspaceGetByHandleOrDeviceDefault` (`kernel_ctxshare.c:127`) to
/// `deviceGetDefaultVASpace(pDevice, ppVAS)` (`vaspace.c:231-241`), i.e. to
/// `pDevice->pVASpace` (`device_share.c:324-347`) — an `OBJVASPACE` with **no
/// `VaSpaceApi` resource and no client handle**.
///
/// That address space nevertheless names itself on the wire **exactly once**, and this is
/// the field that identifies the message. In
/// `gvaspaceCopyServerRmReservedPdesToServerRm_IMPL`
/// (`ogkm-580: src/nvidia/src/kernel/mem_mgr/gpu_vaspace.c:4066-4136`), when the calling
/// resource is not a `VaSpaceApi` the local `hVASpace` is **zero** (`:4070-4075`), and on
/// a GSP client RM then:
///
/// 1. mints a fresh handle in the same client — `serverutilGenResourceHandle(hClient,
///    &hVASpace)` (`:4101`);
/// 2. sets `vaParams.index = NV_VASPACE_ALLOCATION_INDEX_GPU_DEVICE` and issues
///    `NV_RM_RPC_ALLOC_OBJECT(… hDevice, hVASpace, FERMI_VASPACE_A, &vaParams …)`
///    (`:4103-4113`) — RM's own comment: *"VAS handle is 0 for the device vaspace. Trigger
///    an allocation on server RM so that the plugin has a valid handle to the device VAS
///    under this client."*;
/// 3. publishes the reserved PDEs at `rmCtrlParams.hObject = hVASpace` (`:4128` → `:5175`);
/// 4. ★★★ **frees the handle again** — `NV_RM_RPC_FREE(pGpu, hClient, hDevice, hVASpace)`
///    (`:4135`), guarded by the `bFreeNeeded` this branch set.
///
/// ⇒ **The free frees the NAME, not the ADDRESS SPACE.** `pDevice->pVASpace` is untouched
/// by step 4 (`deviceRemoveFromClientShare_IMPL`, `device_share.c:307-320`, is the only
/// thing that destroys it, and it runs when the *Device* goes away). A port that treats
/// step 4 as destroying the VA space loses the only statement of it the wire ever carried
/// — which is precisely what §16.25–§16.27 measured as `NO-VASPACE-IN-NAMESPACE`.
///
/// ⊘ It is *not* a fact we could have observed better: §16.25 concluded the intermediate
/// is unobservable by construction, and it is right about the wrapper TSG. It was wrong
/// about the VA space, which is on the wire, under this constant.
pub const NV_VASPACE_ALLOCATION_INDEX_GPU_DEVICE: u32 = 0x03;

/// `NV_VASPACE_ALLOCATION_PARAMETERS` —
/// `ogkm-580: src/common/sdk/nvidia/inc/nvos.h:3154-3164`.
///
/// The alloc parameters of `FERMI_VASPACE_A`, i.e. of *"a fresh host GPU virtual address
/// space for one guest `Vas`"* — the per-`Vas` separation that is #14's proven fix. All
/// zeroes is a legal and meaningful request: index 0, no flags, and a zero `va_size` means
/// *"the whole default range"*, which is what an isolate wants.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NvVaspaceAllocationParameters {
    /// `NvU32 index` @ +0.
    pub index: u32,
    /// `NvV32 flags` @ +4.
    pub flags: u32,
    /// `NvU64 vaSize` @ +8 — 0 means "the default range".
    pub va_size: u64,
    /// `NvU64 vaStartInternal` @ +16.
    pub va_start_internal: u64,
    /// `NvU64 vaLimitInternal` @ +24.
    pub va_limit_internal: u64,
    /// `NvU32 bigPageSize` @ +32 — 0 means "the system default".
    pub big_page_size: u32,
    // +36: four bytes of padding before the 8-aligned `vaBase`.
    /// `NvU64 vaBase` @ +40.
    pub va_base: u64,
    /// `NvU32 pasid` @ +48.
    pub pasid: u32,
    // +52: four bytes of tail padding to the struct's 8-byte alignment.
}

impl NvVaspaceAllocationParameters {
    /// The C typedef name.
    pub const C_NAME: &'static str = "NV_VASPACE_ALLOCATION_PARAMETERS";
    /// `sizeof`.
    pub const SIZE: usize = 56;
    /// `alignof`.
    pub const ALIGN: usize = 8;

    /// Encode into a little-endian image of at least [`Self::SIZE`] bytes. Padding and any
    /// tail are left as found.
    ///
    /// # Errors
    /// [`AbiError::Truncated`].
    pub fn encode_into(&self, bytes: &mut [u8]) -> Result<(), AbiError> {
        put(
            bytes,
            Self::C_NAME,
            Self::SIZE,
            0,
            &self.index.to_le_bytes(),
        )?;
        put(
            bytes,
            Self::C_NAME,
            Self::SIZE,
            4,
            &self.flags.to_le_bytes(),
        )?;
        put(
            bytes,
            Self::C_NAME,
            Self::SIZE,
            8,
            &self.va_size.to_le_bytes(),
        )?;
        put(
            bytes,
            Self::C_NAME,
            Self::SIZE,
            16,
            &self.va_start_internal.to_le_bytes(),
        )?;
        put(
            bytes,
            Self::C_NAME,
            Self::SIZE,
            24,
            &self.va_limit_internal.to_le_bytes(),
        )?;
        put(
            bytes,
            Self::C_NAME,
            Self::SIZE,
            32,
            &self.big_page_size.to_le_bytes(),
        )?;
        put(
            bytes,
            Self::C_NAME,
            Self::SIZE,
            40,
            &self.va_base.to_le_bytes(),
        )?;
        put(
            bytes,
            Self::C_NAME,
            Self::SIZE,
            48,
            &self.pasid.to_le_bytes(),
        )
    }
}

/// `nv_ioctl_nvos02_parameters_with_fd` — `NVOS02_PARAMETERS`
/// (`ogkm-580: src/common/sdk/nvidia/inc/nvos.h:283-293`) followed by the frontend's
/// descriptor field, which is what [`NV_ESC_RM_ALLOC_MEMORY`] actually carries
/// (`C: src/abi/nvgpu.h:243-256`, 56 bytes).
///
/// ## ★ Why memory allocation has its own escape at all
///
/// `NV_ESC_RM_ALLOC` cannot express it: the reply has to carry a **descriptor** as well as
/// a handle (for the mapping that follows), and an `NVOS21` has nowhere to put one. So
/// sysmem allocation is a different ioctl with a different struct, and a backend that
/// treats "allocate memory" as just another `alloc` will find the class refused rather
/// than the parameters misread — which is the good failure, but only if you know why.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Nvos02ParametersWithFd {
    /// `NvHandle hRoot` @ +0.
    pub h_root: u32,
    /// `NvHandle hObjectParent` @ +4.
    pub h_object_parent: u32,
    /// `NvHandle hObjectNew` @ +8 — in *and* out.
    pub h_object_new: u32,
    /// `NvV32 hClass` @ +12.
    pub h_class: u32,
    /// `NvV32 flags` @ +16 — the `NVOS02_FLAGS_*` bitfield.
    pub flags: u32,
    // +20: four bytes of padding before the 8-aligned `pMemory`.
    /// `NvP64 pMemory` @ +24 — a host CPU address when the caller is describing existing
    /// memory. **This crate never fills it in**; a backend that needs to must route the
    /// address through `kayfabe_linux_raw::Indirect`, which is the only place one may
    /// exist.
    pub p_memory: u64,
    /// `NvU64 limit` @ +32 — the allocation size **minus one**. Off-by-one by ABI, not by
    /// mistake: it is a limit, not a length.
    pub limit: u64,
    /// `NvV32 status` @ +40 — `[OUT]`.
    pub status: u32,
    /// `NvU32 pad1` @ +44 — RM's own tail padding.
    ///
    /// Named, and **deliberately not written by [`Self::encode_into`]**. It exists so that
    /// rustc places [`Self::fd`] at +48; a struct without it would put `fd` at +44, which is
    /// inside `NVOS02_PARAMETERS` rather than after it. Naming it also makes a struct
    /// literal that forgets it a compile error rather than a silent re-layout.
    pub pad1: u32,
    /// `int fd` @ **+48** — `[OUT]` the frontend's descriptor for the new object.
    ///
    /// ★ **Not +44.** `NVOS02_PARAMETERS` ends at `status` (+40) and is 48 bytes with its
    /// tail padding; the frontend's `nv_ioctl_nvos02_parameters_with_fd` embeds that whole
    /// struct and appends `fd` **after** it. Putting `fd` at +44 writes into RM's padding
    /// and leaves the frontend reading a zero — i.e. descriptor 0, which is stdin.
    ///
    /// ★ **Read off two sources, not measured.** This said "Measured against", and the two
    /// things it was measured against are a header and a translation table — both text.
    /// The offset agrees in `C: src/abi/nvgpu.h:243-256` and in the C stub's own table, which
    /// rewrites the NVOS02 descriptor at **offset 48** (`C: src/stub/nvkvm_stub.c:1149-1174`).
    /// Two independent readings that agree, which is worth stating and is still not a run:
    /// nobody has watched a descriptor of 0 come back from a live frontend.
    pub fd: i32,
    // +52: four bytes of tail padding; the ioctl's declared size is 56.
}

impl Nvos02ParametersWithFd {
    /// The C typedef name.
    pub const C_NAME: &'static str = "nv_ioctl_nvos02_parameters_with_fd";
    /// The size the ioctl request number declares.
    pub const SIZE: usize = 56;
    /// `alignof`.
    pub const ALIGN: usize = 8;
    /// ★ Byte offset of [`Self::p_memory`] — the argument's one pointer field, and the
    /// number a [`kayfabe_linux_raw::Indirect`] needs.
    ///
    /// A named constant rather than a literal at the call site because the only consumer is
    /// an `Indirect`, whose whole job is to write eight bytes at an offset: a wrong offset
    /// there does not fail to compile, it patches an address over `flags` or `limit` and the
    /// driver refuses with something that names neither.
    pub const P_MEMORY_OFFSET: usize = 24;

    /// Decode from a little-endian image of at least [`Self::SIZE`] bytes.
    ///
    /// # Errors
    /// [`AbiError::Truncated`].
    pub fn decode(bytes: &[u8]) -> Result<Self, AbiError> {
        if bytes.len() < Self::SIZE {
            return Err(AbiError::Truncated {
                c_name: Self::C_NAME,
                need: Self::SIZE,
                got: bytes.len(),
            });
        }
        Ok(Self {
            h_root: u32_at(bytes, 0)?,
            h_object_parent: u32_at(bytes, 4)?,
            h_object_new: u32_at(bytes, 8)?,
            h_class: u32_at(bytes, 12)?,
            flags: u32_at(bytes, 16)?,
            p_memory: u64_at(bytes, 24)?,
            limit: u64_at(bytes, 32)?,
            status: u32_at(bytes, 40)?,
            pad1: u32_at(bytes, 44)?,
            fd: u32_at(bytes, 48)? as i32,
        })
    }

    /// Encode into a little-endian image of at least [`Self::SIZE`] bytes.
    ///
    /// # Errors
    /// [`AbiError::Truncated`].
    pub fn encode_into(&self, bytes: &mut [u8]) -> Result<(), AbiError> {
        let n = Self::C_NAME;
        let s = Self::SIZE;
        put(bytes, n, s, 0, &self.h_root.to_le_bytes())?;
        put(bytes, n, s, 4, &self.h_object_parent.to_le_bytes())?;
        put(bytes, n, s, 8, &self.h_object_new.to_le_bytes())?;
        put(bytes, n, s, 12, &self.h_class.to_le_bytes())?;
        put(bytes, n, s, 16, &self.flags.to_le_bytes())?;
        put(bytes, n, s, 24, &self.p_memory.to_le_bytes())?;
        put(bytes, n, s, 32, &self.limit.to_le_bytes())?;
        put(bytes, n, s, 40, &self.status.to_le_bytes())?;
        put(bytes, n, s, 48, &self.fd.to_le_bytes())
    }
}

/// ★★ `NVOS02_FLAGS_LOCATION_PCI` — system memory, i.e. across the bus rather than in
/// VRAM. Field `11:8`, **value 0** (`ogkm-580: src/common/sdk/nvidia/inc/nvos.h`).
///
/// ## The mistake this constant is the corrected form of — found on hardware
///
/// NVIDIA's `NVOS02_FLAGS_*` headers give a **field range** (`11:8`) and a **value within
/// it** (`_PCI` = 0, `_VIDMEM` = 2). They are not bit masks, and reading them as masks is
/// the obvious wrong move: the first real-hardware run of this crate sent
/// `1 << 10` — which is value **4** in field `11:8`, a location that does not exist — and
/// RM answered `NV_ERR_INVALID_FLAGS` (0x29). No mock could have said so; the flag word is
/// exactly the kind of value a double accepts because it never looks at it.
///
/// So every constant here is `value << field_lsb`, and `_PCI` being **zero** is not an
/// omission — it is what "system memory" encodes as.
pub const NVOS02_FLAGS_LOCATION_PCI: u32 = 0 << 8;

/// `NVOS02_FLAGS_PHYSICALITY_NONCONTIGUOUS` — field `7:4`, value 1.
///
/// Non-contiguous is the right default and the choice is not cosmetic: demanding contiguous
/// system memory is how an allocation that worked at bring-up fails after an hour of
/// fragmentation, which is the least reproducible failure available.
pub const NVOS02_FLAGS_PHYSICALITY_NONCONTIGUOUS: u32 = 1 << 4;

/// `NVOS02_FLAGS_MAPPING_NO_MAP` — field `31:30`, value 1.
///
/// The isolate maps GPU-side, never CPU-side, at this rung. Asking RM not to map into user
/// space is both correct and one fewer mapping to reclaim — and it is what stops the
/// frontend building an `mmap` context around the descriptor field
/// (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/escape.c:342-345`).
pub const NVOS02_FLAGS_MAPPING_NO_MAP: u32 = 1 << 30;

/// ★★ `NVOS02_FLAGS_COHERENCY_CACHED` — field `15:12`, value 1
/// (`ogkm-580: src/common/sdk/nvidia/inc/nvos.h:196-198`), i.e. `0x1000`.
///
/// The coherency the GPU is told to assume for memory the CPU also writes. **Cached** is
/// the one that makes a plain store from this process visible to a snooped PCIe read
/// without an explicit flush, which is the whole reason
/// [`NV01_MEMORY_SYSTEM_OS_DESCRIPTOR`] is worth having: the isolate writes the bytes
/// with an ordinary `mov` and the engine reads them.
///
/// ⚠ **The field has six values and `_CACHED` (1) is not `_WRITE_BACK` (5).** They are
/// different rows of the same enum and both are plausible names for "the normal one".
/// The C's proven call sends `0x1000` — value 1 — captured from a real host CUDA
/// `cuMemHostAlloc` on 580.159.04 (`C: nvkvm_gpu_emul.c:7519-7524`, flags `0x40001010`),
/// so this is the value hardware has actually accepted and not the one the name suggests.
pub const NVOS02_FLAGS_COHERENCY_CACHED: u32 = 1 << 12;

/// ★★★ `NVOS46_FLAGS_DMA_OFFSET_FIXED_TRUE` — field `15:15`, value 1
/// (`ogkm-580: src/common/sdk/nvidia/inc/nvos.h:2094-2096`), i.e. `0x8000`.
///
/// **The one flag address identity rests on** (`#102`). For a VASpace (non-CTXDMA) map
/// target, `NVOS46_PARAMETERS::dmaOffset` is an **[OUT]** parameter by default — RM picks
/// the GPU VA and tells you where it put it. With this bit set it becomes **[IN]**: RM
/// places the mapping at the address you name (`C: nvkvm_gpu_emul.c:7663-7692`).
///
/// That difference is the whole data plane. A forwarded pushbuffer carries the *guest's*
/// virtual addresses, and the host GPU's MMU walks the host VAS for exactly those numbers.
/// Let the driver choose, and the mapping exists somewhere the guest never names: the
/// submission looks published and faults the instant hardware resolves it
/// (`Xid 31 FAULT_PDE`).
pub const NVOS46_FLAGS_DMA_OFFSET_FIXED_TRUE: u32 = 1 << 15;

/// ★★★★★ `NVOS46_FLAGS_DEFER_TLB_INVALIDATION_TRUE` — field `31:31`, value 1
/// (`ogkm-580: src/common/sdk/nvidia/inc/nvos.h:2149-2151`), i.e. `0x8000_0000`.
///
/// **The flag that lets a client map memory and skip the TLB invalidate.** With it set,
/// `dmaAllocMapping_GM107` takes `DMA_DEFER_TLB_INVALIDATE` instead of `DMA_TLB_INVALIDATE`
/// (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/arch/maxwell/virt_mem_allocator_gm107.c:417`)
/// and the gate at the function's `done:` label — `kbusFlush_HAL` +
/// `gvaspaceInvalidateTlb` (`:2610-2615`) — is not taken. The PTE is written; nothing is
/// told about it.
///
/// # ⊘ WHAT THE HEADER CLAIMS, AND WHY IT IS NOT AN ANSWER
///
/// The SDK's own warning (`:2144-2148`) names only one hazard: *"Improper use can leave
/// **stale entries** in the TLB, and allow access to memory no longer owned by the RM
/// client or cause page faults."* Stale entries are an **unmap** hazard. Read literally
/// that leans toward *"a fresh PTE goes live on the next walk"* — but RM and UVM both
/// invalidate on a fresh **upgrade** anyway, and `uvm_mmu.c:805-808` says in as many words
/// that an upgrade needs no membar and then still issues `tlb_invalidate_all`, which would
/// be dead work if a walk always picked the new entry up.
///
/// ⇒ **Source does not settle it.** This constant exists so `kayfabe-rm-ladder`'s
/// `--defer-liveness` rung can ask hardware instead, against a VA whose non-present result
/// was already walked and cached. Nothing in the forwarding plane sets it.
pub const NVOS46_FLAGS_DEFER_TLB_INVALIDATION_TRUE: u32 = 1 << 31;

/// `NVOS47_FLAGS_DEFER_TLB_INVALIDATION_TRUE` — field `0:0`, value 1, on the UNMAP escape
/// (`ogkm-580: src/common/sdk/nvidia/inc/nvos.h:2190-2192`). ⚠ A different bit from the map's
/// `31:31`: the two escapes name the same idea at different positions.
pub const NVOS47_FLAGS_DEFER_TLB_INVALIDATION_TRUE: u32 = 1;

/// ★★ `NVOS46_FLAGS_PAGE_SIZE_4KB` — field `11:8`, value 1
/// (`ogkm-580: src/common/sdk/nvidia/inc/nvos.h:2036-2038`), i.e. `0x0000_0100`.
///
/// Pins a mapping to the **small-page** table rather than letting `_dmaGetPageSize` pick.
/// ⊘ Not a performance knob and not a default anything should acquire: it exists so a
/// diagnostic can state *which page-table level it is talking about* and have two mappings
/// provably land in the same leaf table. `PAGE_SIZE_DEFAULT` (0) lets RM choose, and a
/// large enough vidmem object at a large enough alignment can be given a huge PTE that
/// lives at the directory level instead — which would silently move a leaf-level question
/// to a different level and answer it there.
///
/// ⚠ It also decides `pageSizeLockMask` on the VA reservation the map path performs
/// (`virt_mem_allocator_gm107.c:988-996`), so two mappings that name the same value pin the
/// same page table and the second cannot be the one that instantiates it.
pub const NVOS46_FLAGS_PAGE_SIZE_4KB: u32 = 1 << 8;

/// `NVOS46_FLAGS_DMA_OFFSET_GROWS_DOWN` — field `14:14`, value 1 (`ogkm-580: nvos.h:2066-2068`).
/// With no FIXED address, RM places the mapping from the TOP of the space down — where a guest
/// kernel's bottom-up allocations are least likely to land (`THE_TRANSLATED_PLANE.md` §24.2).
pub const NVOS46_FLAGS_DMA_OFFSET_GROWS_DOWN: u32 = 1 << 14;

/// ★★★ **THE BIG-PAGE SIZE THIS ARCHITECTURE FAMILY USES — 64 KiB.**
///
/// ⊘ **Not a per-die constant, and constraint 12 is the reason the distinction is written
/// down rather than assumed.** Fermi through Blackwell all present 64 KiB as
/// `NV_VASPACE_BIG_PAGE_SIZE_64K`, and [`NvVaspaceAllocationParameters::big_page_size`] is
/// `0` on every space this crate allocates, which means *"the family default"*. So this is
/// a **family** fact, which is the maintainable form constraint 12 names — not a GA106
/// measurement. ⚠ If a space is ever allocated asking for 128 KiB, this number stops being
/// the right one and [`nvos46_page_size_flag`] must take the space's own value instead.
pub const NVOS46_BIG_PAGE_BYTES: u64 = 64 * 1024;

/// ★★★★★ **WHICH `NVOS46_FLAGS_PAGE_SIZE` A *FIXED* MAP OF `[offset, offset+len)` AT `at`
/// MUST CARRY — constraint 28, and it is a measurement, not a preference.**
///
/// > ### ⊘⊘⊘⊘ REFUTED AS A DIAGNOSIS, SAME DAY, BY THE OWNER — **THE FIX IS RIGHT AND THE
/// > ### REASON GIVEN FOR IT WAS WRONG.** Read this before the block below.
/// >
/// > Owner, 2026-09-16: *"I don't think RM is going to randomly switch page alignment
/// > requirements… how was it unaligned? or is this the wrong understanding?"*
/// >
/// > **It is the wrong understanding, and the check is one line of provenance.** The slices
/// > this predicate decides for arrive from `map_store_slice_for_leaf`, which passes
/// > `leaf.len` and `leaf.phys` straight out of a **page-table walk** — `len` is *"the walk's
/// > own page size for this entry, never an assumed one"* and `phys` is the frame that entry
/// > names. **A PTE's physical base is aligned to its own page size by hardware.** So
/// > `offset` can never be LESS aligned than `len` requires:
/// >
/// > | leaf | `len` | `at` | `offset` | old predicate |
/// > |---|---|---|---|---|
/// > | 4 KiB | `0x1000` | any | any | already `_4KB` — `len` fails the test |
/// > | 64 KiB | `0x10000` | 64K-aligned | 64K-aligned | `0`, and correctly so |
/// > | 2 MiB | `0x200000` | 2M-aligned | 2M-aligned | `0`, and correctly so |
/// >
/// > ⇒ **The case the third quantity was added for does not arise on this path**, and this
/// > is therefore NOT the cause of `map_refused=2154`. The `[measured w744]` rows below are
/// > real, but they were measured on **ring VAs the isolate itself chose**, not on walked
/// > guest leaves. Carrying that conclusion to a different provenance without re-checking the
/// > premise is the same error this file's own corrections keep naming.
/// >
/// > ★ **The change is KEPT, on a narrower and honest argument**: the predicate is now total
/// > over its own inputs instead of resting on an alignment invariant established in another
/// > crate by a walker it cannot see. A caller that ever maps a sub-leaf slice, or a store
/// > offset from any source but a PTE, gets the right flag rather than a silent relocation.
/// > ⊘ It buys correctness under a wider set of callers; it does not explain the boot.
/// >
/// > ⚠ **THE WALL IS UNATTRIBUTED AGAIN.** What the w755 `refusals=[…]` histogram
/// > discriminates, all three now visible because `want`/`got` survive the wire:
/// > **delta constant and large** ⇒ `dmaOffset` read relative to the `NV01_MEMORY_VIRTUAL`
/// > range's base while we pass an absolute guest VA; **delta sub-page and varying** ⇒ a
/// > page-size or kind disagreement after all; **`got == 0`** ⇒ `FIXED` ignored on this path.
/// >
/// > ### ⊘⊘⊘ THE SUPERSEDED REASONING, kept because the CHANGE it produced is still in force
/// >
/// > This function took `(at, len)`. The argument below — *"a large leaf is necessarily at a
/// > large-aligned VA, so it can be served by a big page at its own address and needs no
/// > flag"* — is sound **for a dedicated leaf**, whose physical base is the allocation's own
/// > base and is therefore aligned by construction. It is **false for a slice**: the single
/// > store maps `[offset, offset+len)` of ONE 11 904 MiB object, and a 64 KiB PTE needs the
/// > **physical** side 64 KiB-aligned too.
/// >
/// > ⚠ A guest's VA alignment and its frame alignment are **independent**. `at` 64 KiB-aligned
/// > with `offset` only 4 KiB-aligned is not a corner case — it is the ordinary shape of a
/// > guest framebuffer run — and in exactly that case the old predicate returned `0`
/// > (*"RM chooses"*), RM chose a big page, could not honour `dmaOffset`, **aligned it down
/// > and answered `NV_OK`**, and constraint 28 refused the relocation.
/// >
/// > ⇒ `[measured w753, split-ownership boot]` `map_refused=2154`. ⚠ **Attributed by
/// > mechanism, not yet by a boot**: the identity of the refusal is measured (constraint 28,
/// > via the w755 wire repair), and this is the mechanism that produces it at this rate. The
/// > confirming row is the `refusals=[…]` histogram showing `want`/`got` deltas **under
/// > 64 KiB** with `want` big-aligned. Recorded as the ranked hypothesis until that row.
/// >
/// > ★ The class: **a correct rule whose premise changed underneath it.** Nothing was wrong
/// > when it was written and nothing edited it; the single store made a third quantity
/// > load-bearing, and a predicate cannot notice that its own subject grew.
///
/// `[measured w744, GA106, driver 580.126.20, `traces/w744_b1d_probe/`]`
/// `NVOS46_FLAGS_DMA_OFFSET_FIXED_TRUE` makes `dmaOffset` an `[IN]` — and RM **still**
/// runs `_dmaGetPageSize`, which is free to choose a big page. A big-page mapping cannot
/// begin on a 4 KiB boundary, so RM **aligns the request down and answers `NV_OK`**:
///
/// ```text
/// at=0x0000008000001000 status=0x0000 dmaOffset=0x0000008000000000 honoured=false
/// at=0x0000009000001000 status=0x0000 dmaOffset=0x0000009000001000 honoured=true  (4K flag)
/// ```
///
/// ⇒ **0/3 of the raw client's own ring VAs were honoured without the flag and 3/3 with
/// it**, through a *success* in both cases. The mapping then exists somewhere the guest
/// never names, which is the `Xid 31 FAULT_PDE`
/// [`NVOS46_FLAGS_DMA_OFFSET_FIXED_TRUE`]'s own docs describe.
///
/// # ★ The rule, and why it is stated on the REQUEST rather than on the guest's page class
///
/// A large leaf is necessarily at a large-aligned VA, so it can be served by a big page at
/// its own address and needs no flag; a request whose VA or length is **not** big-aligned
/// cannot be served by a big page at all, so pinning it to the small-page table is the only
/// way `FIXED` can be honoured. ⇒ the predicate is a property of `(at, len)`, which is
/// available at **every** map site, rather than of a `Run::class` that only the (as yet
/// unbuilt) run publisher holds. ⊘ The two agree by construction — the coalescer never
/// emits a 64 KiB-class run at a VA that is not 64 KiB-aligned — so this is the same rule
/// read off the numbers that are actually in hand.
/// ⊘⊘ **w755: `(at, len)` is now `(at, offset, len)` — see the correction at the top. The
/// sentence above is still the right argument; it was applied to a quantity list that was
/// complete for a leaf and short by one for a slice.**
///
/// ⊘ `PAGE_SIZE_DEFAULT` is `0`, i.e. *"RM chooses"*, and is returned deliberately rather
/// than a transcribed `_BIG`: the only member of this field this crate has read the header
/// for is `_4KB`, and naming a value we have not transcribed would be a guess wearing a
/// constant's clothes.
#[must_use]
pub const fn nvos46_page_size_flag(at: u64, offset: u64, len: u64) -> u32 {
    // ⊘ ALL THREE. A big page needs the VA, the LENGTH and the PHYSICAL BASE big-aligned;
    // the physical base of a slice is the reservation's base (big-aligned) plus `offset`.
    if at % NVOS46_BIG_PAGE_BYTES == 0
        && offset % NVOS46_BIG_PAGE_BYTES == 0
        && len % NVOS46_BIG_PAGE_BYTES == 0
    {
        0
    } else {
        NVOS46_FLAGS_PAGE_SIZE_4KB
    }
}

/// ★★★★★ **THE PAGE-SIZE FLAG A *STORE SLICE* MUST CARRY, AND IT IS UNCONDITIONAL.
/// DERIVED FROM ogkm's SOURCE, NOT MEASURED AND NOT GUESSED.**
///
/// # The rule RM actually applies — congruence, not alignment
///
/// `[ogkm-580.159.04, virt_mem_allocator_gm107.c:726, :1081, :1532]`, with
/// `virtual_mem.c:1323` supplying the descriptor:
///
/// ```text
///   virtual_mem.c:1323   memdescCreateSubMem(&pDmaMappingInfo->pMemDesc, pSrcMemDesc,
///                                            pGpu, offset, length);
///   gm107.c:726          pageOffset = memdescGetPhysAddr(pTempMemDesc, at, 0)
///                                       & (pageSize - 1);
///   gm107.c:922          vaLo       = RM_ALIGN_DOWN(*pVaddr, pageSize);
///   gm107.c:1081         if ((*pVaddr - vaLo) != 0 && (*pVaddr - vaLo) != pageOffset)
///                            -> NV_ERR_INVALID_OFFSET
///   gm107.c:1532         *pVaddr = vaLo + pageOffset;
/// ```
///
/// ⇒ the NVOS46 `offset` becomes a **sub-descriptor**, so `pageOffset` is
/// `(reservation_base + offset) & (pageSize - 1)`, and RM returns
///
/// ```text
///   got = ALIGN_DOWN(at, pageSize) + pageOffset
///   got == at   ⟺   at ≡ (reservation_base + offset)   (mod pageSize)
/// ```
///
/// ⊘⊘⊘ **So RM does not require the request to be ALIGNED. It requires the VA and the
/// PHYSICAL address to be CONGRUENT modulo the page size it picks.** A perfectly
/// big-page-aligned `at` whose physical side has a non-zero `pageOffset` **passes the
/// consistency check** — it takes the `(*pVaddr - vaLo) == 0` branch — and comes back
/// **relocated by exactly `pageOffset`, reporting `NV_OK`**. That is the silent relocation
/// constraint 28 exists to catch, and neither [`nvos46_page_size_flag`]'s original
/// `(at, len)` form nor its w755 `(at, offset, len)` form expresses it.
///
/// # ★★★★★ WHERE THE MISALIGNMENT COMES FROM — **WE INSERTED IT**
///
/// > Owner, 2026-09-16: *"I suspect the guest already enforces this, so why did this
/// > misalignment fire. The guest driver is not going to violate its own hardware."*
///
/// **It does enforce it, and that is exactly why the fault is ours.** A guest PTE means
/// `at ≡ guest_gpga (mod guest_page_size)` by construction — the guest's relation is intact
/// and was never in question. But the single store's identity is *framebuffer address = file
/// offset*, so what reaches RM is
///
/// ```text
///   physical = reservation_base + guest_gpga
///   RM asks:   at ≡ (reservation_base + guest_gpga)   (mod pageSize)
///   guest gives: at ≡                  guest_gpga     (mod guest_page_size)
///   the difference:  reservation_base mod pageSize
/// ```
///
/// ⇒ `reservation_base` is **our** host allocation's base, a number the guest has never seen
/// and cannot account for. **We added a term to a congruence the guest had already
/// satisfied.** Nothing on the guest side is wrong; its physical side was shifted by an
/// opaque constant underneath it.
///
/// ★★★ **And this predicts the shape of the failure, not just its existence**: every slice is
/// displaced by the *same* `reservation_base mod pageSize`, so the refusals should be **one
/// bucket with a constant delta**, and the count should be *every attempt* rather than some
/// of them. `[measured w753]` `map_refused=2154` with a single `first_refusal` is consistent
/// with that and does not yet confirm it — the confirming row is the w755 `refusals=[…]`
/// histogram showing ONE key. ⊘ Varied deltas would refute this reading, and that is the
/// point of recording it before the boot.
///
/// # ★ Why this is `_4KB` ON THE NONCONTIGUOUS RESERVATION, and 'RM chooses' on the other
///
/// ⊘ w755c: this was unconditional. It is now conditioned on the reservation actually coming
/// back contiguous and 1 GiB-aligned, because that is what makes `phys ≡ offset` true — see
/// `RmConnection::reserve_gpga`. Pinning 4 KiB on a store where the congruence holds anyway
/// costs TLB reach for nothing.
///
/// ⊘ The term above vanishes at 4 KiB, and that — not conservatism — is the argument.
/// `reservation_base` is RM's choice and **we never learn it**: `RmBackend::reserve_gpga`
/// answers a `HostHandle` and nothing else. So congruence at any size above 4 KiB is
/// **unprovable from here**. At 4 KiB it is free: `at` comes from a guest PTE and
/// `reservation_base + offset` is at worst 4 KiB-granular, so `pageOffset == 0`,
/// `vaLo == at`, and `got == at` **exactly**, for every slice, wherever RM put the
/// reservation.
///
/// ⚠ **This costs TLB reach and the cost is real**: the whole guest framebuffer is mapped
/// with 4 KiB PTEs. It is taken deliberately — a relocated mapping is an `Xid 31 FAULT_PDE`
/// and a wrong answer, a small page is a slow correct one. ⊘ The optimisation is named
/// rather than hand-waved: **learn the reservation's GPGA, then pick the largest page size
/// satisfying the congruence above**, and measure the TLB difference rather than assuming
/// it. Until `reserve_gpga` reports that address, there is nothing to compute with.
///
/// ⊘ **Scoped to store slices on purpose.** Compressed kinds require big pages, and a
/// blanket `_4KB` across every map site could be refused outright by RM — a different and
/// worse failure. [`nvos46_page_size_flag`] keeps serving callers whose object base IS the
/// mapping base.
#[must_use]
pub const fn nvos46_page_size_flag_for_store_slice(contiguous_and_aligned: bool) -> u32 {
    // ⊘⊘⊘⊘ **MEASURED WRONG AND REVERTED — w755e, 4 633 relocations in one boot.**
    //
    // This took the argument and answered `0` ("RM chooses") when the reservation came back
    // contiguous and 1 GiB-aligned, on the argument that congruence then holds at every page
    // size so the pin costs TLB reach for nothing. **The congruence reasoning is right and the
    // conclusion is false**, because honouring `DMA_OFFSET_FIXED` was never only about
    // congruence:
    //
    // `[measured w755e, route-K arm]` with the pin removed,
    //   `maps=1 map_refused=4633`, every one CONSTRAINT 28, every `got = want + k*0x10000`
    //   with `k` marching 1,4,5,6,7,8,9,a,b,c… — **one allocator counter handing out the next
    //   free 64 KiB slot across unrelated bases.** That is not a congruence failure, which
    //   would be a FIXED per-base offset; it is `FIXED` being ignored outright.
    // `[measured w744]` said the same thing first: `DMA_OFFSET_FIXED_TRUE` alone honoured
    //   **0 of 3** ring VAs, and **3 of 3** with the small-page flag.
    //
    // ⇒ the flag is not a congruence remedy, it is what makes RM honour the address at all.
    // The argument is kept in the parameter rather than deleted, so the next person who
    // reasons their way to the same conclusion meets the measurement instead of repeating it.
    let _ = contiguous_and_aligned;
    NVOS46_FLAGS_PAGE_SIZE_4KB
}

/// Bounds-checked field write, shared by every `encode_into` above.
fn put(
    bytes: &mut [u8],
    c_name: &'static str,
    need: usize,
    off: usize,
    src: &[u8],
) -> Result<(), AbiError> {
    let got = bytes.len();
    bytes
        .get_mut(off..off + src.len())
        .ok_or(AbiError::Truncated { c_name, need, got })?
        .copy_from_slice(src);
    Ok(())
}

// The transcriptions vs rustc, at COMPILE time — the same gate the generated structs get.
const _: () = {
    assert!(core::mem::size_of::<RegisterFd>() == RegisterFd::SIZE);
    assert!(
        core::mem::size_of::<NvMemoryVirtualAllocationParams>()
            == NvMemoryVirtualAllocationParams::SIZE
    );
    assert!(core::mem::offset_of!(NvMemoryVirtualAllocationParams, offset) == 0);
    assert!(core::mem::offset_of!(NvMemoryVirtualAllocationParams, limit) == 8);
    assert!(core::mem::offset_of!(NvMemoryVirtualAllocationParams, h_va_space) == 16);
    assert!(core::mem::size_of::<Nv2080AllocParameters>() == Nv2080AllocParameters::SIZE);

    assert!(
        core::mem::size_of::<NvVaspaceAllocationParameters>()
            == NvVaspaceAllocationParameters::SIZE
    );
    assert!(
        core::mem::align_of::<NvVaspaceAllocationParameters>()
            == NvVaspaceAllocationParameters::ALIGN
    );
    assert!(core::mem::offset_of!(NvVaspaceAllocationParameters, index) == 0);
    assert!(core::mem::offset_of!(NvVaspaceAllocationParameters, flags) == 4);
    assert!(core::mem::offset_of!(NvVaspaceAllocationParameters, va_size) == 8);
    assert!(core::mem::offset_of!(NvVaspaceAllocationParameters, va_start_internal) == 16);
    assert!(core::mem::offset_of!(NvVaspaceAllocationParameters, va_limit_internal) == 24);
    assert!(core::mem::offset_of!(NvVaspaceAllocationParameters, big_page_size) == 32);
    assert!(core::mem::offset_of!(NvVaspaceAllocationParameters, va_base) == 40);
    assert!(core::mem::offset_of!(NvVaspaceAllocationParameters, pasid) == 48);

    // ★ NOT `size_of == SIZE`. The declared ioctl size is 56 and rustc computes 56 only
    // because the explicit `_pad1` puts `fd` where the frontend reads it; the OFFSETS are
    // what must agree, and the size assertion is left off deliberately so a future field
    // cannot be "fixed" by padding until the number matches.
    assert!(core::mem::align_of::<Nvos02ParametersWithFd>() == Nvos02ParametersWithFd::ALIGN);
    assert!(core::mem::offset_of!(Nvos02ParametersWithFd, h_root) == 0);
    assert!(core::mem::offset_of!(Nvos02ParametersWithFd, h_object_parent) == 4);
    assert!(core::mem::offset_of!(Nvos02ParametersWithFd, h_object_new) == 8);
    assert!(core::mem::offset_of!(Nvos02ParametersWithFd, h_class) == 12);
    assert!(core::mem::offset_of!(Nvos02ParametersWithFd, flags) == 16);
    assert!(core::mem::offset_of!(Nvos02ParametersWithFd, p_memory) == 24);
    assert!(core::mem::offset_of!(Nvos02ParametersWithFd, limit) == 32);
    assert!(core::mem::offset_of!(Nvos02ParametersWithFd, status) == 40);
    assert!(core::mem::offset_of!(Nvos02ParametersWithFd, pad1) == 44);
    assert!(core::mem::offset_of!(Nvos02ParametersWithFd, fd) == 48);
};

#[cfg(test)]
mod tests {
    use super::*;

    /// The round trip, on the one struct that has both directions. `p_memory` is
    /// deliberately included: this crate must be able to *carry* the field even though it
    /// must never *mint* the value.
    #[test]
    fn nvos02_with_fd_round_trips_every_declared_field() {
        let p = Nvos02ParametersWithFd {
            h_root: 0xC1D0_0001,
            h_object_parent: 0xC1D0_0002,
            h_object_new: 0xCAFE_0007,
            h_class: NV01_MEMORY_SYSTEM,
            flags: NVOS02_FLAGS_LOCATION_PCI | NVOS02_FLAGS_MAPPING_NO_MAP,
            p_memory: 0,
            limit: 0x0FFF,
            status: 0,
            pad1: 0,
            fd: -1,
        };
        let mut bytes = [0u8; Nvos02ParametersWithFd::SIZE];
        p.encode_into(&mut bytes).expect("encode");
        assert_eq!(Nvos02ParametersWithFd::decode(&bytes).expect("decode"), p);
    }

    /// ★★ [`Nvos02ParametersWithFd::P_MEMORY_OFFSET`] names the field an `Indirect`
    /// patches, and the way to check it is to **encode a marker and find it there** —
    /// not to compare the constant with the literal `24` it was written from, which is
    /// the same reading twice.
    #[test]
    fn the_pointer_field_offset_is_where_the_encoder_puts_p_memory() {
        const MARKER: u64 = 0x0BAD_C0DE_DEAD_BEEF;
        let mut bytes = [0u8; Nvos02ParametersWithFd::SIZE];
        Nvos02ParametersWithFd {
            p_memory: MARKER,
            ..Default::default()
        }
        .encode_into(&mut bytes)
        .expect("encode");
        let at = Nvos02ParametersWithFd::P_MEMORY_OFFSET;
        assert_eq!(
            u64::from_le_bytes(bytes[at..at + 8].try_into().expect("eight bytes")),
            MARKER,
            "an `Indirect` writing at P_MEMORY_OFFSET must land on `pMemory`"
        );
        // And the offset is inside the struct with a whole pointer's room after it, which
        // is the bound `CharDevice::ioctl` checks the patch against.
        assert!(at + 8 <= Nvos02ParametersWithFd::SIZE);
    }

    /// The padding is not written, so a caller that put something there keeps it — the
    /// same rule the generated `encode_into` follows.
    #[test]
    fn encoding_leaves_declared_padding_untouched() {
        let mut bytes = [0xEEu8; Nvos02ParametersWithFd::SIZE];
        Nvos02ParametersWithFd::default()
            .encode_into(&mut bytes)
            .expect("encode");
        assert_eq!(&bytes[20..24], &[0xEE; 4], "the pre-`pMemory` padding");
        // ★ `pad1` is a NAMED field and is still not written. It exists to make rustc place
        // `fd` at +48; the encoder's rule is "declared fields only, padding as found", and
        // padding does not stop being padding because it has a name.
        assert_eq!(&bytes[44..48], &[0xEE; 4], "RM's tail padding, before `fd`");
        assert_eq!(&bytes[52..56], &[0xEE; 4], "the wrapper's tail");
    }

    #[test]
    fn a_short_buffer_is_refused_with_the_exact_variant() {
        let mut small = [0u8; 8];
        assert_eq!(
            Nvos02ParametersWithFd::default().encode_into(&mut small),
            Err(AbiError::Truncated {
                c_name: Nvos02ParametersWithFd::C_NAME,
                need: Nvos02ParametersWithFd::SIZE,
                got: 8,
            })
        );
        assert_eq!(
            Nvos02ParametersWithFd::decode(&small),
            Err(AbiError::Truncated {
                c_name: Nvos02ParametersWithFd::C_NAME,
                need: Nvos02ParametersWithFd::SIZE,
                got: 8,
            })
        );
    }

    #[test]
    fn the_vaspace_parameters_encode_at_the_declared_offsets() {
        let p = NvVaspaceAllocationParameters {
            index: 1,
            flags: 2,
            va_size: 0x1_0000_0000,
            va_start_internal: 0x10,
            va_limit_internal: 0x20,
            big_page_size: 0x2_0000,
            va_base: 0x30,
            pasid: 7,
        };
        let mut bytes = [0u8; NvVaspaceAllocationParameters::SIZE];
        p.encode_into(&mut bytes).expect("encode");
        assert_eq!(u32_at(&bytes, 0), Ok(1));
        assert_eq!(u32_at(&bytes, 4), Ok(2));
        assert_eq!(u64_at(&bytes, 8), Ok(0x1_0000_0000));
        assert_eq!(u64_at(&bytes, 16), Ok(0x10));
        assert_eq!(u64_at(&bytes, 24), Ok(0x20));
        assert_eq!(u32_at(&bytes, 32), Ok(0x2_0000));
        assert_eq!(u64_at(&bytes, 40), Ok(0x30));
        assert_eq!(u32_at(&bytes, 48), Ok(7));
    }

    /// ★ The flag constants are `value << field_lsb`, not bit masks. Pinned by the field
    /// each one must land in, because the mask reading is what a real driver rejected.
    #[test]
    fn nvos02_flags_encode_a_value_into_their_field() {
        // LOCATION is 11:8 and PCI is value 0 — so the field must be ZERO, and a
        // "one bit per name" reading would put a 1 somewhere in 11:8.
        assert_eq!(NVOS02_FLAGS_LOCATION_PCI, 0);
        assert_eq!(
            (NVOS02_FLAGS_LOCATION_PCI >> 8) & 0xF,
            0,
            "LOCATION_PCI = 0"
        );
        // PHYSICALITY is 7:4, NONCONTIGUOUS is value 1.
        assert_eq!((NVOS02_FLAGS_PHYSICALITY_NONCONTIGUOUS >> 4) & 0xF, 1);
        assert_eq!(
            NVOS02_FLAGS_PHYSICALITY_NONCONTIGUOUS & !0xF0,
            0,
            "no spill"
        );
        // MAPPING is 31:30, NO_MAP is value 1.
        assert_eq!((NVOS02_FLAGS_MAPPING_NO_MAP >> 30) & 0x3, 1);
        // And the three together occupy three disjoint fields.
        let all = NVOS02_FLAGS_LOCATION_PCI
            | NVOS02_FLAGS_PHYSICALITY_NONCONTIGUOUS
            | NVOS02_FLAGS_MAPPING_NO_MAP;
        assert_eq!(all, 0x4000_0010);
        // COHERENCY is 15:12, CACHED is value 1 — and it is value 1, NOT the `_WRITE_BACK`
        // row (5) whose name reads like the same thing.
        assert_eq!((NVOS02_FLAGS_COHERENCY_CACHED >> 12) & 0xF, 1);
        assert_eq!(NVOS02_FLAGS_COHERENCY_CACHED & !0xF000, 0, "no spill");
        // ★ The exact word the C sends and a real 580.159.04 accepted, reassembled from
        // this crate's four constants. A constant that drifts into the wrong field breaks
        // this equality rather than surfacing as `NV_ERR_INVALID_FLAGS` on hardware.
        assert_eq!(
            NVOS02_FLAGS_LOCATION_PCI
                | NVOS02_FLAGS_PHYSICALITY_NONCONTIGUOUS
                | NVOS02_FLAGS_COHERENCY_CACHED
                | NVOS02_FLAGS_MAPPING_NO_MAP,
            0x4000_1010,
            "C: nvkvm_gpu_emul.c:7519-7524"
        );
    }

    /// The two escape spaces really are disjoint numbering schemes over one magic — the
    /// fact the module docs warn about, asserted so a future edit that "tidies" one of
    /// them into the other fails here.
    #[test]
    fn the_frontend_and_rm_escape_spaces_do_not_collide() {
        use crate::generated::nvos::{NV_ESC_RM_ALLOC, NV_ESC_RM_CONTROL, NV_ESC_RM_FREE};
        let frontend = [NV_ESC_REGISTER_FD, NV_ESC_CHECK_VERSION_STR];
        let rm = [NV_ESC_RM_FREE, NV_ESC_RM_CONTROL, NV_ESC_RM_ALLOC];
        for f in frontend {
            assert!(
                !rm.contains(&u32::from(f)),
                "frontend escape {f:#x} collides with an RM escape"
            );
        }
        assert_eq!(NV_ESC_REGISTER_FD, 200 + 1);
        assert_eq!(NV_ESC_CHECK_VERSION_STR, 200 + 10);
    }

    /// ★ `nv_ioctl_card_info_t`'s offsets, pinned by a `#[repr(C)]` mirror of the C header
    /// (`nv-ioctl.h:31-38,54-66`) rather than by the hand-written numbers in `decode` — two
    /// statements of one layout that must agree.
    #[test]
    fn card_info_layout_matches_the_c_header() {
        #[repr(C)]
        struct PciInfo {
            domain: u32,
            bus: u8,
            slot: u8,
            function: u8,
            vendor_id: u16,
            device_id: u16,
        }
        #[repr(C)]
        struct Ci {
            valid: u8,
            pci_info: PciInfo,
            gpu_id: u32,
            interrupt_line: u16,
            reg_address: u64,
            reg_size: u64,
            fb_address: u64,
            fb_size: u64,
            minor_number: u32,
            dev_name: [u8; 10],
        }
        assert_eq!(core::mem::size_of::<Ci>(), CardInfo::SIZE);
        assert_eq!(core::mem::offset_of!(Ci, pci_info), 4);
        assert_eq!(core::mem::offset_of!(Ci, pci_info) + core::mem::offset_of!(PciInfo, bus), 8);
        assert_eq!(core::mem::offset_of!(Ci, pci_info) + core::mem::offset_of!(PciInfo, slot), 9);
        assert_eq!(core::mem::offset_of!(Ci, pci_info) + core::mem::offset_of!(PciInfo, device_id), 14);
        assert_eq!(core::mem::offset_of!(Ci, gpu_id), 16);
        assert_eq!(core::mem::offset_of!(Ci, reg_size), 32);
        assert_eq!(core::mem::offset_of!(Ci, minor_number), 56);

        let mut b = vec![0u8; CardInfo::SIZE * 3];
        // entry 0: minor 1 on bus 0x41; entry 1: minor 0 on bus 0x01; entry 2 invalid.
        for (i, (bus, minor, gpu)) in [(0x41u8, 1u32, 0x4100u32), (0x01, 0, 0x100)].iter().enumerate() {
            let o = i * CardInfo::SIZE;
            b[o] = 1;
            b[o + 8] = *bus;
            b[o + 16..o + 20].copy_from_slice(&gpu.to_le_bytes());
            b[o + 56..o + 60].copy_from_slice(&minor.to_le_bytes());
        }
        let all = CardInfo::decode_all(&b).expect("decode");
        assert_eq!(all.len(), 2, "the zeroed tail entry is not a GPU");
        assert_eq!((all[0].minor, all[0].bus, all[0].gpu_id), (1, 0x41, 0x4100));
        assert_eq!((all[1].minor, all[1].bus, all[1].gpu_id), (0, 0x01, 0x100));
        assert_eq!(NV_ESC_CARD_INFO, 200);
    }

    #[test]
    fn gpu_id_info_v2_request_and_reply() {
        let mut b = [0xAAu8; GpuIdInfoV2::SIZE];
        GpuIdInfoV2::encode_request(0x4100, &mut b).expect("encode");
        assert_eq!(&b[0..4], &0x4100u32.to_le_bytes());
        assert!(b[4..].iter().all(|&x| x == 0), "every out field starts zero");
        b[8..12].copy_from_slice(&0u32.to_le_bytes());
        b[12..16].copy_from_slice(&0u32.to_le_bytes());
        let r = GpuIdInfoV2::decode(&b).expect("decode");
        assert_eq!((r.gpu_id, r.device_instance, r.sub_device_instance), (0x4100, 0, 0));
    }
}
