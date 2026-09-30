//! ★★★ **The host `/dev/nvidia-uvm` ABI kf3 speaks in EFS mode** — the stock UVM ioctls an
//! EFS-mode twin needs, and the b3 patch's `UVM_EFS_*` ioctls.
//!
//! `docs/design/V3_UVM_GUEST_FAULT_PLANE.md` §2, §3.0. Every offset and size below was derived by
//! COMPILING the headers with `gcc` and `offsetof`/`sizeof` on 2026-09-30 — `ogkm-580:
//! kernel-open/nvidia-uvm/uvm_ioctl.h`, `uvm_linux_ioctl.h`, `kernel-open/common/inc` — and the
//! patch's own `tools/uvm_efs/patch/uvm_efs_ioctl.h` (ABI version 1). The tests pin them.
//!
//! ⊘ **The request numbers are NOT `_IOC`-encoded.** `UVM_IOCTL_BASE(i)` is `i` on Linux
//! (`uvm_ioctl.h`), so their `_IOC_SIZE` decodes to nothing or to garbage (`UVM_INITIALIZE =
//! 0x30000001` decodes to 12 288). The driver copies ITS OWN `sizeof` in both directions, so the
//! buffer handed to the kernel must be at least [`UvmOp::kernel_size`] — which is why the raw
//! issue path (`kf_linux_raw::uvm`) keeps its own table and refuses a shorter buffer.
//!
//! ⊘ **The EFS record's enums are nvidia-uvm INTERNAL enums** (`uvm_hal_types.h:245-355` at
//! 580.159.04), copied raw by the v1 patch. They are pinned here ([`UvmAccessType`],
//! [`UvmFaultType`], [`UvmClientType`]) for the one host module version the patch exists for; a
//! v2 ABI should carry EFS-defined constants instead (`V3_UVM_GUEST_FAULT_PLANE.md` §10 Q2).

use crate::wire::{AbiError, u32_at, u64_at};

/// `UVM_INITIALIZE` (`uvm_linux_ioctl.h:32`).
pub const UVM_INITIALIZE: u64 = 0x3000_0001;
/// `UVM_REGISTER_GPU_VASPACE` = `UVM_IOCTL_BASE(25)`.
pub const UVM_REGISTER_GPU_VASPACE: u64 = 25;
/// `UVM_UNREGISTER_GPU_VASPACE` = `UVM_IOCTL_BASE(26)`.
pub const UVM_UNREGISTER_GPU_VASPACE: u64 = 26;
/// `UVM_REGISTER_CHANNEL` = `UVM_IOCTL_BASE(27)`.
pub const UVM_REGISTER_CHANNEL: u64 = 27;
/// `UVM_UNREGISTER_CHANNEL` = `UVM_IOCTL_BASE(28)`.
pub const UVM_UNREGISTER_CHANNEL: u64 = 28;
/// `UVM_MAP_EXTERNAL_ALLOCATION` = `UVM_IOCTL_BASE(33)`.
pub const UVM_MAP_EXTERNAL_ALLOCATION: u64 = 33;
/// `UVM_FREE` = `UVM_IOCTL_BASE(34)`.
pub const UVM_FREE: u64 = 34;
/// `UVM_REGISTER_GPU` = `UVM_IOCTL_BASE(37)`.
pub const UVM_REGISTER_GPU: u64 = 37;
/// `UVM_UNREGISTER_GPU` = `UVM_IOCTL_BASE(38)`.
pub const UVM_UNREGISTER_GPU: u64 = 38;
/// `UVM_UNMAP_EXTERNAL` = `UVM_IOCTL_BASE(66)`.
pub const UVM_UNMAP_EXTERNAL: u64 = 66;
/// `UVM_CREATE_EXTERNAL_RANGE` = `UVM_IOCTL_BASE(73)`.
pub const UVM_CREATE_EXTERNAL_RANGE: u64 = 73;
/// `UVM_EFS_QUERY` = `UVM_IOCTL_BASE(1800)` (`uvm_efs_ioctl.h`).
pub const UVM_EFS_QUERY: u64 = 1800;
/// `UVM_EFS_WAIT` = `UVM_IOCTL_BASE(1801)`.
pub const UVM_EFS_WAIT: u64 = 1801;
/// `UVM_EFS_RESOLVE` = `UVM_IOCTL_BASE(1802)`.
pub const UVM_EFS_RESOLVE: u64 = 1802;

/// `UVM_INIT_FLAGS_DISABLE_HMM` (`uvm_types.h`).
pub const UVM_INIT_FLAGS_DISABLE_HMM: u64 = 0x1;
/// `UVM_INIT_FLAGS_MULTI_PROCESS_SHARING_MODE`.
pub const UVM_INIT_FLAGS_MULTI_PROCESS_SHARING_MODE: u64 = 0x2;
/// `UVM_INIT_FLAGS_EXTERNAL_FAULT_SERVICE` (`uvm_efs_ioctl.h`) — outside `UVM_INIT_FLAGS_MASK`, so a
/// stock module refuses it with `NV_ERR_INVALID_ARGUMENT` and the patched one, disabled, with
/// `NV_ERR_NOT_SUPPORTED`: the two are distinguishable.
pub const UVM_INIT_FLAGS_EXTERNAL_FAULT_SERVICE: u64 = 0x4000_0000_0000_0000;

/// The EFS ABI version this side speaks.
pub const UVM_EFS_ABI_VERSION: u32 = 1;
/// `UVM_EFS_ACTION_REPLAY`.
pub const UVM_EFS_ACTION_REPLAY: u32 = 1;
/// `UVM_EFS_ACTION_CANCEL`.
pub const UVM_EFS_ACTION_CANCEL: u32 = 2;
/// `UVM_EFS_MAX_WAIT_RECORDS`.
pub const UVM_EFS_MAX_WAIT_RECORDS: u32 = 64;
/// `UVM_EFS_MAX_RESOLVE_RECORDS`.
pub const UVM_EFS_MAX_RESOLVE_RECORDS: u32 = 256;
/// `UVM_EFS_MAX_WAIT_TIMEOUT_US`.
pub const UVM_EFS_MAX_WAIT_TIMEOUT_US: u32 = 1_000_000;
/// `UVM_EFS_CTR_COUNT`.
pub const UVM_EFS_CTR_COUNT: usize = 12;
/// `UVM_EFS_GCTR_COUNT`.
pub const UVM_EFS_GCTR_COUNT: usize = 4;

/// `NV_ERR_NOT_SUPPORTED`.
pub const NV_ERR_NOT_SUPPORTED: u32 = 0x56;
/// `NV_ERR_INVALID_ARGUMENT`.
pub const NV_ERR_INVALID_ARGUMENT: u32 = 0x1f;
/// `NV_ERR_INSUFFICIENT_PERMISSIONS`.
pub const NV_ERR_INSUFFICIENT_PERMISSIONS: u32 = 0x1b;

/// `UVM_MAX_GPUS` at 580.159.04 = `NV_MAX_DEVICES (32) * UVM_PARENT_ID_MAX_SUB_PROCESSORS (8)`.
pub const UVM_MAX_GPUS: usize = 256;
/// `sizeof(UvmGpuMappingAttributes)`.
pub const MAPPING_ATTRIBUTES_SIZE: usize = 36;

/// `UvmGpuMappingType` (`nv_uvm_user_types.h:67-70`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum MappingType {
    /// `UvmGpuMappingTypeDefault`.
    Default = 0,
    /// `UvmGpuMappingTypeReadWriteAtomic`.
    ReadWriteAtomic = 1,
    /// `UvmGpuMappingTypeReadWrite` — no atomics.
    ReadWrite = 2,
    /// `UvmGpuMappingTypeReadOnly`.
    ReadOnly = 3,
}

/// `UvmGpuCachingType` (`nv_uvm_user_types.h:86-88`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CachingType {
    /// `UvmGpuCachingTypeDefault`.
    Default = 0,
    /// `UvmGpuCachingTypeForceUncached`.
    ForceUncached = 1,
    /// `UvmGpuCachingTypeForceCached`.
    ForceCached = 2,
}

/// One UVM operation kf3 issues: its request number and the size of the kernel's parameter
/// struct at 580.159.04 (the size the driver copies in and out).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UvmOp {
    /// `UVM_INITIALIZE`.
    Initialize,
    /// `UVM_REGISTER_GPU`.
    RegisterGpu,
    /// `UVM_UNREGISTER_GPU`.
    UnregisterGpu,
    /// `UVM_REGISTER_GPU_VASPACE`.
    RegisterGpuVaSpace,
    /// `UVM_UNREGISTER_GPU_VASPACE`.
    UnregisterGpuVaSpace,
    /// `UVM_REGISTER_CHANNEL`.
    RegisterChannel,
    /// `UVM_UNREGISTER_CHANNEL`.
    UnregisterChannel,
    /// `UVM_CREATE_EXTERNAL_RANGE`.
    CreateExternalRange,
    /// `UVM_MAP_EXTERNAL_ALLOCATION`.
    MapExternalAllocation,
    /// `UVM_UNMAP_EXTERNAL`.
    UnmapExternal,
    /// `UVM_FREE`.
    Free,
    /// `UVM_EFS_QUERY`.
    EfsQuery,
    /// `UVM_EFS_WAIT`.
    EfsWait,
    /// `UVM_EFS_RESOLVE`.
    EfsResolve,
}

impl UvmOp {
    /// Every op.
    pub const ALL: [UvmOp; 14] = [
        UvmOp::Initialize,
        UvmOp::RegisterGpu,
        UvmOp::UnregisterGpu,
        UvmOp::RegisterGpuVaSpace,
        UvmOp::UnregisterGpuVaSpace,
        UvmOp::RegisterChannel,
        UvmOp::UnregisterChannel,
        UvmOp::CreateExternalRange,
        UvmOp::MapExternalAllocation,
        UvmOp::UnmapExternal,
        UvmOp::Free,
        UvmOp::EfsQuery,
        UvmOp::EfsWait,
        UvmOp::EfsResolve,
    ];

    /// The request number.
    #[must_use]
    pub const fn request(self) -> u64 {
        match self {
            UvmOp::Initialize => UVM_INITIALIZE,
            UvmOp::RegisterGpu => UVM_REGISTER_GPU,
            UvmOp::UnregisterGpu => UVM_UNREGISTER_GPU,
            UvmOp::RegisterGpuVaSpace => UVM_REGISTER_GPU_VASPACE,
            UvmOp::UnregisterGpuVaSpace => UVM_UNREGISTER_GPU_VASPACE,
            UvmOp::RegisterChannel => UVM_REGISTER_CHANNEL,
            UvmOp::UnregisterChannel => UVM_UNREGISTER_CHANNEL,
            UvmOp::CreateExternalRange => UVM_CREATE_EXTERNAL_RANGE,
            UvmOp::MapExternalAllocation => UVM_MAP_EXTERNAL_ALLOCATION,
            UvmOp::UnmapExternal => UVM_UNMAP_EXTERNAL,
            UvmOp::Free => UVM_FREE,
            UvmOp::EfsQuery => UVM_EFS_QUERY,
            UvmOp::EfsWait => UVM_EFS_WAIT,
            UvmOp::EfsResolve => UVM_EFS_RESOLVE,
        }
    }

    /// `sizeof` the kernel's parameter struct (gcc, 2026-09-30, 580.159.04 + EFS v1).
    #[must_use]
    pub const fn kernel_size(self) -> usize {
        match self {
            UvmOp::Initialize => 16,
            UvmOp::RegisterGpu => 40,
            UvmOp::UnregisterGpu => 20,
            UvmOp::RegisterGpuVaSpace => 32,
            UvmOp::UnregisterGpuVaSpace => 20,
            UvmOp::RegisterChannel => 56,
            UvmOp::UnregisterChannel => 28,
            UvmOp::CreateExternalRange => 24,
            UvmOp::MapExternalAllocation => 9264,
            UvmOp::UnmapExternal => 40,
            UvmOp::Free => 24,
            UvmOp::EfsQuery => 168,
            UvmOp::EfsWait => 24,
            UvmOp::EfsResolve => 32,
        }
    }

    /// Where the kernel writes `rmStatus` in the struct.
    #[must_use]
    pub const fn status_offset(self) -> usize {
        match self {
            UvmOp::Initialize => 8,
            UvmOp::RegisterGpu => 36,
            UvmOp::UnregisterGpu | UvmOp::UnregisterGpuVaSpace | UvmOp::CreateExternalRange => 16,
            UvmOp::RegisterGpuVaSpace => 28,
            UvmOp::RegisterChannel => 48,
            UvmOp::UnregisterChannel => 24,
            UvmOp::MapExternalAllocation => 9260,
            UvmOp::UnmapExternal => 32,
            UvmOp::Free => 16,
            UvmOp::EfsQuery => 160,
            UvmOp::EfsWait => 20,
            UvmOp::EfsResolve => 24,
        }
    }

    /// The name, for a refusal.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            UvmOp::Initialize => "UVM_INITIALIZE",
            UvmOp::RegisterGpu => "UVM_REGISTER_GPU",
            UvmOp::UnregisterGpu => "UVM_UNREGISTER_GPU",
            UvmOp::RegisterGpuVaSpace => "UVM_REGISTER_GPU_VASPACE",
            UvmOp::UnregisterGpuVaSpace => "UVM_UNREGISTER_GPU_VASPACE",
            UvmOp::RegisterChannel => "UVM_REGISTER_CHANNEL",
            UvmOp::UnregisterChannel => "UVM_UNREGISTER_CHANNEL",
            UvmOp::CreateExternalRange => "UVM_CREATE_EXTERNAL_RANGE",
            UvmOp::MapExternalAllocation => "UVM_MAP_EXTERNAL_ALLOCATION",
            UvmOp::UnmapExternal => "UVM_UNMAP_EXTERNAL",
            UvmOp::Free => "UVM_FREE",
            UvmOp::EfsQuery => "UVM_EFS_QUERY",
            UvmOp::EfsWait => "UVM_EFS_WAIT",
            UvmOp::EfsResolve => "UVM_EFS_RESOLVE",
        }
    }

    /// The `rmStatus` the kernel wrote into `buf`.
    ///
    /// # Errors
    /// [`AbiError::Truncated`] if `buf` is shorter than the kernel's struct.
    pub fn status(self, buf: &[u8]) -> Result<u32, AbiError> {
        if buf.len() < self.kernel_size() {
            return Err(AbiError::Truncated {
                c_name: self.name(),
                need: self.kernel_size(),
                got: buf.len(),
            });
        }
        u32_at(buf, self.status_offset())
    }

    /// A zeroed parameter buffer of exactly the kernel's size.
    #[must_use]
    pub fn buffer(self) -> Vec<u8> {
        vec![0u8; self.kernel_size()]
    }
}

fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn put64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

/// A 16-byte processor UUID (`NvProcessorUuid`).
pub type Uuid = [u8; 16];

/// `UVM_INITIALIZE_PARAMS { u64 flags; NV_STATUS rmStatus; }`.
#[must_use]
pub fn initialize(flags: u64) -> Vec<u8> {
    let mut b = UvmOp::Initialize.buffer();
    put64(&mut b, 0, flags);
    b
}

/// `UVM_REGISTER_GPU_PARAMS` — `rmCtrlFd = -1` and `hClient = 0` ask UVM to use its own RM
/// session (`uvm_ioctl.h:534-543`; the b3 tests' `uvm_register_gpu` does the same).
#[must_use]
pub fn register_gpu(uuid: &Uuid) -> Vec<u8> {
    let mut b = UvmOp::RegisterGpu.buffer();
    b[0..16].copy_from_slice(uuid);
    put32(&mut b, 24, u32::MAX); // rmCtrlFd = -1
    b
}

/// `UVM_UNREGISTER_GPU_PARAMS`.
#[must_use]
pub fn unregister_gpu(uuid: &Uuid) -> Vec<u8> {
    let mut b = UvmOp::UnregisterGpu.buffer();
    b[0..16].copy_from_slice(uuid);
    b
}

/// `UVM_REGISTER_GPU_VASPACE_PARAMS` — our control fd, our client, our VA space.
#[must_use]
pub fn register_gpu_vaspace(uuid: &Uuid, rm_ctl_fd: i32, client: u32, vaspace: u32) -> Vec<u8> {
    let mut b = UvmOp::RegisterGpuVaSpace.buffer();
    b[0..16].copy_from_slice(uuid);
    put32(&mut b, 16, rm_ctl_fd as u32);
    put32(&mut b, 20, client);
    put32(&mut b, 24, vaspace);
    b
}

/// `UVM_UNREGISTER_GPU_VASPACE_PARAMS`.
#[must_use]
pub fn unregister_gpu_vaspace(uuid: &Uuid) -> Vec<u8> {
    let mut b = UvmOp::UnregisterGpuVaSpace.buffer();
    b[0..16].copy_from_slice(uuid);
    b
}

/// `UVM_REGISTER_CHANNEL_PARAMS` — our channel; `[base, base+length)` is where UVM may place the
/// channel's resources (GR context buffers).
#[must_use]
pub fn register_channel(
    uuid: &Uuid,
    rm_ctl_fd: i32,
    client: u32,
    channel: u32,
    base: u64,
    length: u64,
) -> Vec<u8> {
    let mut b = UvmOp::RegisterChannel.buffer();
    b[0..16].copy_from_slice(uuid);
    put32(&mut b, 16, rm_ctl_fd as u32);
    put32(&mut b, 20, client);
    put32(&mut b, 24, channel);
    put64(&mut b, 32, base);
    put64(&mut b, 40, length);
    b
}

/// `UVM_UNREGISTER_CHANNEL_PARAMS`.
#[must_use]
pub fn unregister_channel(uuid: &Uuid, client: u32, channel: u32) -> Vec<u8> {
    let mut b = UvmOp::UnregisterChannel.buffer();
    b[0..16].copy_from_slice(uuid);
    put32(&mut b, 16, client);
    put32(&mut b, 20, channel);
    b
}

/// `UVM_CREATE_EXTERNAL_RANGE_PARAMS`.
#[must_use]
pub fn create_external_range(base: u64, length: u64) -> Vec<u8> {
    let mut b = UvmOp::CreateExternalRange.buffer();
    put64(&mut b, 0, base);
    put64(&mut b, 8, length);
    b
}

/// `UVM_FREE_PARAMS`.
#[must_use]
pub fn free(base: u64, length: u64) -> Vec<u8> {
    let mut b = UvmOp::Free.buffer();
    put64(&mut b, 0, base);
    put64(&mut b, 8, length);
    b
}

/// One GPU's attributes of an external mapping (`UvmGpuMappingAttributes`); format, element bits
/// and compression stay default (the allocation's own kind).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapAttrs {
    /// The GPU.
    pub uuid: Uuid,
    /// Access.
    pub mapping: MappingType,
    /// Caching.
    pub caching: CachingType,
}

/// `UVM_MAP_EXTERNAL_ALLOCATION_PARAMS` for ONE GPU: map `[offset, offset+length)` of our RM
/// object `memory` at `base`.
#[must_use]
pub fn map_external_allocation(
    base: u64,
    length: u64,
    offset: u64,
    attrs: &MapAttrs,
    rm_ctl_fd: i32,
    client: u32,
    memory: u32,
) -> Vec<u8> {
    let mut b = UvmOp::MapExternalAllocation.buffer();
    put64(&mut b, 0, base);
    put64(&mut b, 8, length);
    put64(&mut b, 16, offset);
    let a = 24;
    b[a..a + 16].copy_from_slice(&attrs.uuid);
    put32(&mut b, a + 16, attrs.mapping as u32);
    put32(&mut b, a + 20, attrs.caching as u32);
    put64(&mut b, 9240, 1); // gpuAttributesCount
    put32(&mut b, 9248, rm_ctl_fd as u32);
    put32(&mut b, 9252, client);
    put32(&mut b, 9256, memory);
    b
}

/// `UVM_UNMAP_EXTERNAL_PARAMS`.
#[must_use]
pub fn unmap_external(base: u64, length: u64, uuid: &Uuid) -> Vec<u8> {
    let mut b = UvmOp::UnmapExternal.buffer();
    put64(&mut b, 0, base);
    put64(&mut b, 8, length);
    b[16..32].copy_from_slice(uuid);
    b
}

/// `UVM_EFS_QUERY_PARAMS`, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EfsQuery {
    /// `abiVersion`.
    pub abi_version: u32,
    /// `moduleEnabled` (`uvm_efs_enable`).
    pub module_enabled: bool,
    /// `active` — THIS file is an EFS VA space.
    pub active: bool,
    /// `maxRecords`.
    pub max_records: u32,
    /// `timeoutMs`.
    pub timeout_ms: u32,
    /// `skipDivertedReplays`.
    pub skip_diverted_replays: bool,
    /// `numParked`.
    pub num_parked: u32,
    /// `numUndelivered`.
    pub num_undelivered: u32,
    /// `vaSpaceCounters[UVM_EFS_CTR_COUNT]`.
    pub counters: [u64; UVM_EFS_CTR_COUNT],
    /// `globalCounters[UVM_EFS_GCTR_COUNT]`.
    pub global: [u64; UVM_EFS_GCTR_COUNT],
}

/// Decode a filled `UVM_EFS_QUERY_PARAMS`.
///
/// # Errors
/// [`AbiError::Truncated`].
pub fn decode_efs_query(b: &[u8]) -> Result<EfsQuery, AbiError> {
    if b.len() < UvmOp::EfsQuery.kernel_size() {
        return Err(AbiError::Truncated {
            c_name: "UVM_EFS_QUERY_PARAMS",
            need: UvmOp::EfsQuery.kernel_size(),
            got: b.len(),
        });
    }
    let mut counters = [0u64; UVM_EFS_CTR_COUNT];
    for (i, c) in counters.iter_mut().enumerate() {
        *c = u64_at(b, 32 + 8 * i)?;
    }
    let mut global = [0u64; UVM_EFS_GCTR_COUNT];
    for (i, c) in global.iter_mut().enumerate() {
        *c = u64_at(b, 128 + 8 * i)?;
    }
    Ok(EfsQuery {
        abi_version: u32_at(b, 0)?,
        module_enabled: u32_at(b, 4)? != 0,
        active: u32_at(b, 8)? != 0,
        max_records: u32_at(b, 12)?,
        timeout_ms: u32_at(b, 16)?,
        skip_diverted_replays: u32_at(b, 20)? != 0,
        num_parked: u32_at(b, 24)?,
        num_undelivered: u32_at(b, 28)?,
        counters,
        global,
    })
}

/// `UVM_EFS_WAIT_PARAMS`, with the `records` pointer field left zero — the raw issue path patches
/// it to the record buffer for the duration of the call ([`EFS_WAIT_RECORDS_PTR_AT`]).
#[must_use]
pub fn efs_wait(max_records: u32, timeout_us: u32) -> Vec<u8> {
    let mut b = UvmOp::EfsWait.buffer();
    put32(&mut b, 8, max_records);
    put32(&mut b, 12, timeout_us);
    b
}

/// Offset of `UVM_EFS_WAIT_PARAMS.records` (a user pointer).
pub const EFS_WAIT_RECORDS_PTR_AT: usize = 0;
/// Offset of `UVM_EFS_WAIT_PARAMS.numRecords` (OUT).
pub const EFS_WAIT_NUM_RECORDS_AT: usize = 16;

/// `UVM_EFS_RESOLVE_PARAMS`, with the `recordIds` pointer field left zero ([`EFS_RESOLVE_IDS_PTR_AT`]).
#[must_use]
pub fn efs_resolve(count: u32, action: u32) -> Vec<u8> {
    let mut b = UvmOp::EfsResolve.buffer();
    put32(&mut b, 8, count);
    put32(&mut b, 12, action);
    b
}

/// Offset of `UVM_EFS_RESOLVE_PARAMS.recordIds` (a user pointer).
pub const EFS_RESOLVE_IDS_PTR_AT: usize = 0;

/// `(numResolved, numStale)` from a filled `UVM_EFS_RESOLVE_PARAMS`.
///
/// # Errors
/// [`AbiError::Truncated`].
pub fn decode_efs_resolve(b: &[u8]) -> Result<(u32, u32), AbiError> {
    Ok((u32_at(b, 16)?, u32_at(b, 20)?))
}

/// `sizeof(UvmEfsFaultRecord)`.
pub const EFS_RECORD_SIZE: usize = 88;

/// `uvm_fault_access_type_t` (`uvm_hal_types.h:245-253`, 580.159.04).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UvmAccessType {
    /// `UVM_FAULT_ACCESS_TYPE_PREFETCH` = 0.
    Prefetch,
    /// `UVM_FAULT_ACCESS_TYPE_READ` = 1.
    Read,
    /// `UVM_FAULT_ACCESS_TYPE_WRITE` = 2.
    Write,
    /// `UVM_FAULT_ACCESS_TYPE_ATOMIC_WEAK` = 3.
    AtomicWeak,
    /// `UVM_FAULT_ACCESS_TYPE_ATOMIC_STRONG` = 4.
    AtomicStrong,
}

impl UvmAccessType {
    /// From the raw enum value, or `None` for one this pin does not know.
    #[must_use]
    pub const fn from_raw(v: u32) -> Option<UvmAccessType> {
        Some(match v {
            0 => UvmAccessType::Prefetch,
            1 => UvmAccessType::Read,
            2 => UvmAccessType::Write,
            3 => UvmAccessType::AtomicWeak,
            4 => UvmAccessType::AtomicStrong,
            _ => return None,
        })
    }
}

/// `uvm_fault_type_t` (`uvm_hal_types.h:316-345`, 580.159.04). ⊘ Only the five kinds a fault
/// SERVICE can resolve are named: EFS diverts only faults stock UVM would mark fatal with
/// `NV_ERR_INVALID_ADDRESS`, and every value from `UVM_FAULT_TYPE_FATAL` (5) up is a fault class
/// stock UVM never services — [`UvmFaultType::from_raw`] answers `None` for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UvmFaultType {
    /// `UVM_FAULT_TYPE_INVALID_PDE` = 0.
    InvalidPde,
    /// `UVM_FAULT_TYPE_INVALID_PTE` = 1.
    InvalidPte,
    /// `UVM_FAULT_TYPE_ATOMIC` = 2.
    Atomic,
    /// `UVM_FAULT_TYPE_WRITE` = 3.
    Write,
    /// `UVM_FAULT_TYPE_READ` = 4.
    Read,
}

impl UvmFaultType {
    /// From the raw enum value; `None` for the fatal classes (≥ `UVM_FAULT_TYPE_FATAL` = 5).
    #[must_use]
    pub const fn from_raw(v: u32) -> Option<UvmFaultType> {
        Some(match v {
            0 => UvmFaultType::InvalidPde,
            1 => UvmFaultType::InvalidPte,
            2 => UvmFaultType::Atomic,
            3 => UvmFaultType::Write,
            4 => UvmFaultType::Read,
            _ => return None,
        })
    }
}

/// `uvm_fault_client_type_t` (`uvm_hal_types.h:351-356`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UvmClientType {
    /// `UVM_FAULT_CLIENT_TYPE_GPC` = 0.
    Gpc,
    /// `UVM_FAULT_CLIENT_TYPE_HUB` = 1.
    Hub,
}

impl UvmClientType {
    /// From the raw enum value.
    #[must_use]
    pub const fn from_raw(v: u32) -> Option<UvmClientType> {
        match v {
            0 => Some(UvmClientType::Gpc),
            1 => Some(UvmClientType::Hub),
            _ => None,
        }
    }
}

/// One `UvmEfsFaultRecord`, decoded. The enum fields stay raw here: whether a value is one the
/// guest can be told is the packet builder's decision (`kf_chip::fault`), and a raw value is what
/// a refusal must print.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EfsRecord {
    /// `recordId` — opaque, kernel-issued.
    pub id: u64,
    /// `faultAddress` — GPU VA, page aligned.
    pub address: u64,
    /// `gpuTimestampNs` — the packet's timestamp.
    pub gpu_timestamp: u64,
    /// `divertTimeNs` — host realtime when parked.
    pub divert_ns: u64,
    /// `gpuUuid`.
    pub gpu_uuid: Uuid,
    /// `accessType` (raw `uvm_fault_access_type_t`).
    pub access_type: u32,
    /// `accessTypeMask`.
    pub access_type_mask: u32,
    /// `faultType` (raw `uvm_fault_type_t`).
    pub fault_type: u32,
    /// `clientType` (raw `uvm_fault_client_type_t`).
    pub client_type: u32,
    /// `clientId` (the packet's raw `CLIENT`).
    pub client_id: u32,
    /// `gpcId`.
    pub gpc_id: u32,
    /// `utlbId`.
    pub utlb_id: u32,
    /// `veId` — the HOST's subcontext.
    pub ve_id: u32,
    /// `mmuEngineId` (the packet's raw `ENGINE_ID`).
    pub mmu_engine_id: u32,
    /// `numInstances` — packets coalesced into this record.
    pub num_instances: u32,
}

/// Decode one record from its 88 bytes.
///
/// # Errors
/// [`AbiError::Truncated`].
pub fn decode_efs_record(b: &[u8]) -> Result<EfsRecord, AbiError> {
    if b.len() < EFS_RECORD_SIZE {
        return Err(AbiError::Truncated {
            c_name: "UvmEfsFaultRecord",
            need: EFS_RECORD_SIZE,
            got: b.len(),
        });
    }
    let mut gpu_uuid = [0u8; 16];
    gpu_uuid.copy_from_slice(&b[32..48]);
    Ok(EfsRecord {
        id: u64_at(b, 0)?,
        address: u64_at(b, 8)?,
        gpu_timestamp: u64_at(b, 16)?,
        divert_ns: u64_at(b, 24)?,
        gpu_uuid,
        access_type: u32_at(b, 48)?,
        access_type_mask: u32_at(b, 52)?,
        fault_type: u32_at(b, 56)?,
        client_type: u32_at(b, 60)?,
        client_id: u32_at(b, 64)?,
        gpc_id: u32_at(b, 68)?,
        utlb_id: u32_at(b, 72)?,
        ve_id: u32_at(b, 76)?,
        mmu_engine_id: u32_at(b, 80)?,
        num_instances: u32_at(b, 84)?,
    })
}

/// Encode a record — the tests' and the mock host's half of [`decode_efs_record`].
#[must_use]
pub fn encode_efs_record(r: &EfsRecord) -> [u8; EFS_RECORD_SIZE] {
    let mut b = [0u8; EFS_RECORD_SIZE];
    put64(&mut b, 0, r.id);
    put64(&mut b, 8, r.address);
    put64(&mut b, 16, r.gpu_timestamp);
    put64(&mut b, 24, r.divert_ns);
    b[32..48].copy_from_slice(&r.gpu_uuid);
    for (at, v) in [
        (48, r.access_type),
        (52, r.access_type_mask),
        (56, r.fault_type),
        (60, r.client_type),
        (64, r.client_id),
        (68, r.gpc_id),
        (72, r.utlb_id),
        (76, r.ve_id),
        (80, r.mmu_engine_id),
        (84, r.num_instances),
    ] {
        put32(&mut b, at, v);
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★ Sizes and status offsets are the compiler's (`gcc offsetof/sizeof`, 2026-09-30, over
    /// `uvm_ioctl.h`, `uvm_linux_ioctl.h` and `uvm_efs_ioctl.h`), not a hand count.
    #[test]
    fn the_kernel_struct_sizes_are_the_compilers() {
        let want: [(UvmOp, usize, usize); 14] = [
            (UvmOp::Initialize, 16, 8),
            (UvmOp::RegisterGpu, 40, 36),
            (UvmOp::UnregisterGpu, 20, 16),
            (UvmOp::RegisterGpuVaSpace, 32, 28),
            (UvmOp::UnregisterGpuVaSpace, 20, 16),
            (UvmOp::RegisterChannel, 56, 48),
            (UvmOp::UnregisterChannel, 28, 24),
            (UvmOp::CreateExternalRange, 24, 16),
            (UvmOp::MapExternalAllocation, 9264, 9260),
            (UvmOp::UnmapExternal, 40, 32),
            (UvmOp::Free, 24, 16),
            (UvmOp::EfsQuery, 168, 160),
            (UvmOp::EfsWait, 24, 20),
            (UvmOp::EfsResolve, 32, 24),
        ];
        for (op, size, st) in want {
            assert_eq!(op.kernel_size(), size, "{}", op.name());
            assert_eq!(op.status_offset(), st, "{}", op.name());
            assert!(op.status_offset() + 4 <= op.kernel_size());
        }
        assert_eq!(UvmOp::ALL.len(), want.len());
        // perGpuAttributes[256] at 24 then an 8-aligned count: 24 + 256 * 36 = 9240.
        assert_eq!(24 + UVM_MAX_GPUS * MAPPING_ATTRIBUTES_SIZE, 9240);
    }

    /// The request numbers are plain integers (`UVM_IOCTL_BASE(i) = i`), one of them the famous
    /// exception.
    #[test]
    fn the_request_numbers() {
        assert_eq!(UVM_INITIALIZE, 0x3000_0001);
        assert_eq!(UVM_EFS_QUERY, 1800);
        assert_eq!(UVM_EFS_WAIT, 1801);
        assert_eq!(UVM_EFS_RESOLVE, 1802);
        let mut seen = std::collections::BTreeSet::new();
        for op in UvmOp::ALL {
            assert!(seen.insert(op.request()), "{} duplicated", op.name());
        }
    }

    /// The encoders put each field at the compiler's offset.
    #[test]
    fn the_encoders_place_fields_at_the_compilers_offsets() {
        let u: Uuid = core::array::from_fn(|i| i as u8 + 1);
        let r = register_channel(&u, 7, 0xc1, 0xc2, 0x1_0000_0000, 0x2000);
        assert_eq!(&r[0..16], &u);
        assert_eq!(u32_at(&r, 16).unwrap(), 7);
        assert_eq!(u32_at(&r, 20).unwrap(), 0xc1);
        assert_eq!(u32_at(&r, 24).unwrap(), 0xc2);
        assert_eq!(u64_at(&r, 32).unwrap(), 0x1_0000_0000);
        assert_eq!(u64_at(&r, 40).unwrap(), 0x2000);
        let v = register_gpu_vaspace(&u, 9, 0xaa, 0xbb);
        assert_eq!(
            (
                u32_at(&v, 16).unwrap(),
                u32_at(&v, 20).unwrap(),
                u32_at(&v, 24).unwrap()
            ),
            (9, 0xaa, 0xbb)
        );
        let m = map_external_allocation(
            0x7000_0000,
            0x1000,
            0x2000,
            &MapAttrs {
                uuid: u,
                mapping: MappingType::ReadOnly,
                caching: CachingType::ForceUncached,
            },
            9,
            0xaa,
            0xcc,
        );
        assert_eq!(m.len(), 9264);
        assert_eq!(u64_at(&m, 0).unwrap(), 0x7000_0000);
        assert_eq!(u64_at(&m, 8).unwrap(), 0x1000);
        assert_eq!(u64_at(&m, 16).unwrap(), 0x2000);
        assert_eq!(&m[24..40], &u);
        assert_eq!(u32_at(&m, 40).unwrap(), 3, "gpuMappingType ReadOnly");
        assert_eq!(u32_at(&m, 44).unwrap(), 1, "gpuCachingType ForceUncached");
        assert_eq!(u64_at(&m, 9240).unwrap(), 1, "one GPU");
        assert_eq!(u32_at(&m, 9248).unwrap(), 9);
        assert_eq!(u32_at(&m, 9252).unwrap(), 0xaa);
        assert_eq!(u32_at(&m, 9256).unwrap(), 0xcc);
        let x = unmap_external(0x10, 0x20, &u);
        assert_eq!(&x[16..32], &u);
        let g = register_gpu(&u);
        assert_eq!(u32_at(&g, 24).unwrap(), u32::MAX, "rmCtrlFd = -1");
    }

    /// A record decodes field for field, and the encoder is its inverse.
    #[test]
    fn a_record_round_trips() {
        let r = EfsRecord {
            id: 0x0001_0000_0000_0003,
            address: 0x7f00_1234_5000,
            gpu_timestamp: 99,
            divert_ns: 100,
            gpu_uuid: [7; 16],
            access_type: 2,
            access_type_mask: 0b110,
            fault_type: 1,
            client_type: 0,
            client_id: 0x2b,
            gpc_id: 3,
            utlb_id: 17,
            ve_id: 0,
            mmu_engine_id: 64,
            num_instances: 4,
        };
        let b = encode_efs_record(&r);
        assert_eq!(decode_efs_record(&b).unwrap(), r);
        assert!(decode_efs_record(&b[..87]).is_err(), "truncated is refused");
    }

    /// The pinned internal enums: the five serviceable fault types, the five access types.
    #[test]
    fn the_uvm_enums_are_pinned() {
        assert_eq!(UvmAccessType::from_raw(0), Some(UvmAccessType::Prefetch));
        assert_eq!(
            UvmAccessType::from_raw(4),
            Some(UvmAccessType::AtomicStrong)
        );
        assert_eq!(UvmAccessType::from_raw(5), None);
        assert_eq!(UvmFaultType::from_raw(4), Some(UvmFaultType::Read));
        assert_eq!(
            UvmFaultType::from_raw(5),
            None,
            "UVM_FAULT_TYPE_FATAL and up"
        );
        assert_eq!(UvmClientType::from_raw(1), Some(UvmClientType::Hub));
        assert_eq!(UvmClientType::from_raw(2), None);
    }

    /// The query decodes, and a short buffer is refused.
    #[test]
    fn the_query_decodes() {
        let mut b = UvmOp::EfsQuery.buffer();
        put32(&mut b, 0, 1);
        put32(&mut b, 4, 1);
        put32(&mut b, 8, 1);
        put32(&mut b, 12, 1024);
        put32(&mut b, 16, 10_000);
        put64(&mut b, 32 + 8 * 4, 5); // DELIVERED
        put64(&mut b, 128, 2);
        let q = decode_efs_query(&b).unwrap();
        assert_eq!(q.abi_version, UVM_EFS_ABI_VERSION);
        assert!(q.module_enabled && q.active);
        assert_eq!((q.max_records, q.timeout_ms), (1024, 10_000));
        assert_eq!(q.counters[4], 5);
        assert_eq!(q.global[0], 2);
        assert!(decode_efs_query(&b[..100]).is_err());
    }
}
