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
}
