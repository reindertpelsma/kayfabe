//! ★★★ **The EFS session — kf3's own UVM va_space for one fault-capable guest VA space.**
//! (`docs/design/V3_UVM_GUEST_FAULT_PLANE.md` §3.0, §3.6, §3.7.)
//!
//! Every verb is AUTHORED: our `/dev/nvidia-uvm` file, our RM control fd, our client, our VA space,
//! our channels, our memory objects. Nothing a guest sent reaches `/dev/nvidia-uvm` except, as
//! data kf3 decided on, a VA to map at (the guest's own VA of the leaf being mirrored — the twin
//! runs in the guest's VA space, so that is the one address that must match) and an offset into
//! OUR store or OUR guest-RAM descriptor.
//!
//! ⊘ The b3 patch exists for ONE host driver (`tools/uvm_efs/patch/nvidia_uvm_efs_580.159.04.patch`)
//! and its v1 records carry that module's internal enums (`kf_abi::uvmefs`), so a session is
//! refused on any other host driver version, by name.

use crate::{HostRm, RmError};
use kf_abi::uvmefs::{
    self as u, EfsQuery, EfsRecord, MapAttrs, UVM_EFS_ABI_VERSION, UVM_INIT_FLAGS_DISABLE_HMM,
    UVM_INIT_FLAGS_EXTERNAL_FAULT_SERVICE, UvmOp,
};
use kf_linux_raw::uvm::UvmFile;

/// The one host driver the b3 patch (EFS ABI 1) is built for.
pub const EFS_HOST_DRIVER: &str = "580.159.04";

/// `NV2080_CTRL_CMD_GPU_GET_GID_INFO` (`ogkm-580: ctrl2080gpu.h:1775`).
const NV2080_CTRL_CMD_GPU_GET_GID_INFO: u32 = 0x2080_014a;
/// `NV2080_GPU_CMD_GPU_GET_GID_FLAGS_FORMAT_BINARY` (`ctrl2080gpu.h:1795`); `TYPE_SHA1` is 0.
const GID_FLAGS_FORMAT_BINARY: u32 = 0x2;
/// `sizeof(NV2080_CTRL_GPU_GET_GID_INFO_PARAMS)` = `index, flags, length` + `data[256]`.
const GID_INFO_SIZE: usize = 12 + 256;

/// Why EFS mode is not available — each one a line in the boot report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EfsRefusal {
    /// The host driver is not the version the patch exists for.
    HostDriver(String),
    /// `/dev/nvidia-uvm` could not be opened.
    NoDevice(String),
    /// `UVM_INITIALIZE` with the EFS flag was refused: a stock module answers
    /// `NV_ERR_INVALID_ARGUMENT` (unknown flag), the patched one with `uvm_efs_enable=0`
    /// `NV_ERR_NOT_SUPPORTED`.
    InitRefused(u32),
    /// The query does not describe an active EFS VA space of the ABI this side speaks.
    Query(String),
    /// Anything else the host said.
    Host(String),
}

impl core::fmt::Display for EfsRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            EfsRefusal::HostDriver(v) => write!(
                f,
                "host driver {v} is not {EFS_HOST_DRIVER}, the one the b3 patch is built for"
            ),
            EfsRefusal::NoDevice(e) => write!(f, "/dev/nvidia-uvm: {e}"),
            EfsRefusal::InitRefused(0x1f) => write!(
                f,
                "UVM_INITIALIZE(EXTERNAL_FAULT_SERVICE) -> NV_ERR_INVALID_ARGUMENT: a STOCK nvidia-uvm (no b3 patch)"
            ),
            EfsRefusal::InitRefused(0x56) => write!(
                f,
                "UVM_INITIALIZE(EXTERNAL_FAULT_SERVICE) -> NV_ERR_NOT_SUPPORTED: the b3 module is loaded with uvm_efs_enable=0"
            ),
            EfsRefusal::InitRefused(s) => {
                write!(f, "UVM_INITIALIZE(EXTERNAL_FAULT_SERVICE) -> {s:#x}")
            }
            EfsRefusal::Query(q) => write!(f, "UVM_EFS_QUERY: {q}"),
            EfsRefusal::Host(h) => write!(f, "{h}"),
        }
    }
}

fn raw(e: &kf_linux_raw::RawError) -> RmError {
    crate::ioctl_error(e)
}

/// Issue a flat UVM op and return its `rmStatus` as an [`RmError`].
fn issue(file: &UvmFile, op: UvmOp, mut buf: Vec<u8>) -> Result<Vec<u8>, RmError> {
    file.ioctl(op.request(), &mut buf).map_err(|e| raw(&e))?;
    let st = op
        .status(&buf)
        .map_err(|_| RmError::Other(crate::ABI_DECODE_FAILED))?;
    crate::status_check(st)?;
    Ok(buf)
}

/// Open a UVM file and initialize it as an EFS VA space (HMM off: no VMM address can ever back a
/// GPU access through it — `V3_UVM_DEMAND_PAGING.md` §7.1).
fn open_efs_file(dev: &kf_linux_raw::DevDir) -> Result<UvmFile, EfsRefusal> {
    let file = UvmFile::open(dev).map_err(|e| EfsRefusal::NoDevice(format!("{e:?}")))?;
    let mut buf = u::initialize(UVM_INIT_FLAGS_EXTERNAL_FAULT_SERVICE | UVM_INIT_FLAGS_DISABLE_HMM);
    file.ioctl(UvmOp::Initialize.request(), &mut buf)
        .map_err(|e| EfsRefusal::Host(format!("UVM_INITIALIZE: {e:?}")))?;
    match UvmOp::Initialize.status(&buf) {
        Ok(0) => Ok(file),
        Ok(s) => Err(EfsRefusal::InitRefused(s)),
        Err(e) => Err(EfsRefusal::Host(format!("UVM_INITIALIZE status: {e:?}"))),
    }
}

fn query_file(file: &UvmFile) -> Result<EfsQuery, EfsRefusal> {
    let mut buf = UvmOp::EfsQuery.buffer();
    file.ioctl(UvmOp::EfsQuery.request(), &mut buf)
        .map_err(|e| EfsRefusal::Host(format!("UVM_EFS_QUERY: {e:?}")))?;
    u::decode_efs_query(&buf).map_err(|e| EfsRefusal::Query(format!("{e:?}")))
}

/// Accept a query only as an active EFS VA space of our ABI, with the module enabled.
fn accept(q: &EfsQuery) -> Result<(), EfsRefusal> {
    if q.abi_version != UVM_EFS_ABI_VERSION {
        return Err(EfsRefusal::Query(format!(
            "abiVersion {} (this side speaks {UVM_EFS_ABI_VERSION})",
            q.abi_version
        )));
    }
    if !q.module_enabled || !q.active {
        return Err(EfsRefusal::Query(format!(
            "moduleEnabled={} active={}",
            q.module_enabled, q.active
        )));
    }
    Ok(())
}

impl HostRm {
    /// ★ The host GPU's UUID (`GPU_GET_GID_INFO`, binary SHA-1 form) — what UVM names a GPU by.
    ///
    /// # Errors
    /// The host's refusal, or a GID that is not 16 bytes.
    pub fn gpu_uuid(&self) -> Result<[u8; 16], RmError> {
        let mut p = [0u8; GID_INFO_SIZE];
        p[4..8].copy_from_slice(&GID_FLAGS_FORMAT_BINARY.to_le_bytes());
        self.raw_control(self.subdevice, NV2080_CTRL_CMD_GPU_GET_GID_INFO, &mut p)?;
        let len = u32::from_le_bytes([p[8], p[9], p[10], p[11]]);
        if len != 16 {
            return Err(RmError::Other(crate::ABI_DECODE_FAILED));
        }
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&p[12..28]);
        Ok(uuid)
    }

    /// ★ Realize-time probe: is EFS mode available on this host? Opens a UVM file with the EFS
    /// flag, queries it, and closes it. `Ok` carries the module's parameters for the boot report.
    ///
    /// # Errors
    /// [`EfsRefusal`], by name — the device then runs with the fault plane off.
    pub fn efs_probe(&self) -> Result<EfsQuery, EfsRefusal> {
        if self.driver_version() != EFS_HOST_DRIVER {
            return Err(EfsRefusal::HostDriver(self.driver_version().to_string()));
        }
        let file = open_efs_file(&self.dev)?;
        let q = query_file(&file)?;
        accept(&q)?;
        Ok(q)
    }

    /// ★ An EFS session over `vaspace` — a `FERMI_VASPACE_A` THIS session allocated fault-capable and
    /// externally owned ([`HostRm::alloc_vaspace_uvm`]): open an EFS UVM file, register the GPU and
    /// the VA space in it.
    ///
    /// # Errors
    /// [`EfsRefusal`], by name; nothing is left registered on a refusal (the file closes).
    pub fn efs_session(&self, vaspace: u32) -> Result<EfsSession, EfsRefusal> {
        if self.driver_version() != EFS_HOST_DRIVER {
            return Err(EfsRefusal::HostDriver(self.driver_version().to_string()));
        }
        let uuid = self
            .gpu_uuid()
            .map_err(|e| EfsRefusal::Host(format!("GPU_GET_GID_INFO: {e:?}")))?;
        let file = open_efs_file(&self.dev)?;
        accept(&query_file(&file)?)?;
        issue(&file, UvmOp::RegisterGpu, u::register_gpu(&uuid))
            .map_err(|e| EfsRefusal::Host(format!("UVM_REGISTER_GPU: {e:?}")))?;
        let ctl_fd = self.ctl.fd_number();
        let client = self.client.raw();
        issue(
            &file,
            UvmOp::RegisterGpuVaSpace,
            u::register_gpu_vaspace(&uuid, ctl_fd, client, vaspace),
        )
        .map_err(|e| EfsRefusal::Host(format!("UVM_REGISTER_GPU_VASPACE {vaspace:#x}: {e:?}")))?;
        Ok(EfsSession {
            file,
            uuid,
            ctl_fd,
            client,
            vaspace,
        })
    }

    /// ★ A `FERMI_VASPACE_A` for an EFS session: `ENABLE_PAGE_FAULTING | IS_EXTERNALLY_OWNED`
    /// (`nvos.h`), so its channels raise REPLAYABLE faults (RM sets the instance block's replay
    /// enables only for a fault-capable space) and RM builds no page tables in it — the UVM file
    /// that registers it owns them. ⊘ No `NV01_MEMORY_VIRTUAL` range and no reservations: RM maps
    /// nothing into such a space, so neither would name anything.
    ///
    /// # Errors
    /// The host's refusal.
    pub fn alloc_vaspace_uvm(&self) -> Result<u32, RmError> {
        use kf_abi::bringup::NvVaspaceAllocationParameters;
        /// `NV_VASPACE_ALLOCATION_FLAGS_IS_EXTERNALLY_OWNED` (`nvos.h`, BIT(3)).
        const IS_EXTERNALLY_OWNED: u32 = 1 << 3;
        /// `NV_VASPACE_ALLOCATION_FLAGS_ENABLE_PAGE_FAULTING` (`nvos.h`, BIT(6)).
        const ENABLE_PAGE_FAULTING: u32 = 1 << 6;
        let mut params = [0u8; NvVaspaceAllocationParameters::SIZE];
        NvVaspaceAllocationParameters {
            flags: IS_EXTERNALLY_OWNED | ENABLE_PAGE_FAULTING,
            ..NvVaspaceAllocationParameters::default()
        }
        .encode_into(&mut params)
        .map_err(|_| RmError::Other(crate::ABI_ENCODE_FAILED))?;
        let want = self.mint();
        let space = self.raw_alloc(
            self.device,
            want,
            kf_abi::invariant_classes::VA_SPACE,
            Some(kf_abi::hostabi::HostParams::Measured(
                &kf_abi::generated::matrix::NV_VASPACE_ALLOCATION_PARAMETERS,
            )),
            &mut params,
        )?;
        self.remember(space, self.device);
        Ok(space)
    }
}

/// ★ One EFS UVM file registered over one of our fault-capable VA spaces.
///
/// ⊘ `Send + Sync` and no lock inside: every verb is one ioctl on the file, which nvidia-uvm
/// serialises itself; the caller must hold no lock another thread blocks on across a call
/// (`THE_CONSTRAINTS.md` 4) — `CharDevice::ioctl` asserts it.
#[derive(Debug)]
pub struct EfsSession {
    file: UvmFile,
    uuid: [u8; 16],
    ctl_fd: i32,
    client: u32,
    vaspace: u32,
}

/// What an external mapping carries, from the guest leaf's permissions (`crate::MapPerm`).
#[must_use]
pub fn map_attrs(uuid: [u8; 16], perm: crate::MapPerm) -> MapAttrs {
    MapAttrs {
        uuid,
        mapping: if perm.read_only {
            u::MappingType::ReadOnly
        } else if perm.atomic_disable {
            u::MappingType::ReadWrite
        } else {
            u::MappingType::ReadWriteAtomic
        },
        caching: if perm.volatile {
            u::CachingType::ForceUncached
        } else {
            u::CachingType::Default
        },
    }
}

impl EfsSession {
    /// The RM VA space this session registered.
    #[must_use]
    pub fn vaspace(&self) -> u32 {
        self.vaspace
    }

    /// The GPU's UUID.
    #[must_use]
    pub fn uuid(&self) -> [u8; 16] {
        self.uuid
    }

    /// `UVM_EFS_QUERY` on this file.
    ///
    /// # Errors
    /// The host's refusal.
    pub fn query(&self) -> Result<EfsQuery, RmError> {
        let buf = issue(&self.file, UvmOp::EfsQuery, UvmOp::EfsQuery.buffer())?;
        u::decode_efs_query(&buf).map_err(|_| RmError::Other(crate::ABI_DECODE_FAILED))
    }

    /// `UVM_CREATE_EXTERNAL_RANGE` over `[base, base+len)`.
    ///
    /// # Errors
    /// The host's refusal.
    pub fn create_range(&self, base: u64, len: u64) -> Result<(), RmError> {
        issue(
            &self.file,
            UvmOp::CreateExternalRange,
            u::create_external_range(base, len),
        )
        .map(|_| ())
    }

    /// `UVM_FREE` of a range this session created (unmaps everything in it).
    ///
    /// # Errors
    /// The host's refusal.
    pub fn free_range(&self, base: u64, len: u64) -> Result<(), RmError> {
        issue(&self.file, UvmOp::Free, u::free(base, len)).map(|_| ())
    }

    /// ★ `UVM_MAP_EXTERNAL_ALLOCATION`: `[offset, offset+len)` of OUR RM object `memory` at `va`.
    /// Replaces whatever of ours was mapped there; returns only after the PTE writes and the TLB
    /// invalidate completed (`uvm_map_external.c`: `uvm_tracker_wait_deinit`).
    ///
    /// # Errors
    /// The host's refusal (`NV_ERR_INVALID_ADDRESS` outside our external ranges).
    pub fn map(
        &self,
        va: u64,
        len: u64,
        memory: u32,
        offset: u64,
        perm: crate::MapPerm,
    ) -> Result<(), RmError> {
        let attrs = map_attrs(self.uuid, perm);
        issue(
            &self.file,
            UvmOp::MapExternalAllocation,
            u::map_external_allocation(va, len, offset, &attrs, self.ctl_fd, self.client, memory),
        )
        .map(|_| ())
    }

    /// `UVM_UNMAP_EXTERNAL` of `[va, va+len)` (splits a mapping straddling an edge).
    ///
    /// # Errors
    /// The host's refusal.
    pub fn unmap(&self, va: u64, len: u64) -> Result<(), RmError> {
        issue(
            &self.file,
            UvmOp::UnmapExternal,
            u::unmap_external(va, len, &self.uuid),
        )
        .map(|_| ())
    }

    /// ★ `UVM_REGISTER_CHANNEL` of OUR channel; UVM maps its resources (GR context buffers) inside
    /// `[base, base+len)` and binds them — the channel is schedulable only after this.
    ///
    /// # Errors
    /// The host's refusal.
    pub fn register_channel(&self, channel: u32, base: u64, len: u64) -> Result<(), RmError> {
        issue(
            &self.file,
            UvmOp::RegisterChannel,
            u::register_channel(&self.uuid, self.ctl_fd, self.client, channel, base, len),
        )
        .map(|_| ())
    }

    /// `UVM_UNREGISTER_CHANNEL` — before the channel is freed.
    ///
    /// # Errors
    /// The host's refusal.
    pub fn unregister_channel(&self, channel: u32) -> Result<(), RmError> {
        issue(
            &self.file,
            UvmOp::UnregisterChannel,
            u::unregister_channel(&self.uuid, self.client, channel),
        )
        .map(|_| ())
    }

    /// `UVM_UNREGISTER_GPU_VASPACE` — before the VA space is freed.
    ///
    /// # Errors
    /// The host's refusal.
    pub fn unregister_vaspace(&self) -> Result<(), RmError> {
        issue(
            &self.file,
            UvmOp::UnregisterGpuVaSpace,
            u::unregister_gpu_vaspace(&self.uuid),
        )
        .map(|_| ())
    }

    /// ★ `UVM_EFS_WAIT`: up to `max` (≤ 64) records, blocking at most `timeout_us` (≤ 1 s).
    ///
    /// # Errors
    /// The host's refusal, or a record that does not decode.
    pub fn wait(&self, max: u32, timeout_us: u32) -> Result<Vec<EfsRecord>, RmError> {
        let max = max.clamp(1, u::UVM_EFS_MAX_WAIT_RECORDS);
        let mut arg = u::efs_wait(max, timeout_us.min(u::UVM_EFS_MAX_WAIT_TIMEOUT_US));
        let mut records = vec![0u8; max as usize * u::EFS_RECORD_SIZE];
        self.file
            .efs_wait(&mut arg, &mut records)
            .map_err(|e| raw(&e))?;
        let st = UvmOp::EfsWait
            .status(&arg)
            .map_err(|_| RmError::Other(crate::ABI_DECODE_FAILED))?;
        crate::status_check(st)?;
        let n = kf_abi::wire::u32_at(&arg, u::EFS_WAIT_NUM_RECORDS_AT)
            .map_err(|_| RmError::Other(crate::ABI_DECODE_FAILED))?
            .min(max) as usize;
        records
            .chunks_exact(u::EFS_RECORD_SIZE)
            .take(n)
            .map(|c| u::decode_efs_record(c).map_err(|_| RmError::Other(crate::ABI_DECODE_FAILED)))
            .collect()
    }

    /// ★ `UVM_EFS_RESOLVE` of up to 256 ids with `action` (`UVM_EFS_ACTION_REPLAY` / `_CANCEL`).
    /// Returns `(numResolved, numStale)`.
    ///
    /// # Errors
    /// The host's refusal.
    pub fn resolve(&self, ids: &[u64], action: u32) -> Result<(u32, u32), RmError> {
        let n = ids.len().min(u::UVM_EFS_MAX_RESOLVE_RECORDS as usize);
        if n == 0 {
            return Ok((0, 0));
        }
        let mut arg = u::efs_resolve(n as u32, action);
        let mut buf: Vec<u8> = ids[..n].iter().flat_map(|i| i.to_le_bytes()).collect();
        self.file
            .efs_resolve(&mut arg, &mut buf)
            .map_err(|e| raw(&e))?;
        let st = UvmOp::EfsResolve
            .status(&arg)
            .map_err(|_| RmError::Other(crate::ABI_DECODE_FAILED))?;
        crate::status_check(st)?;
        u::decode_efs_resolve(&arg).map_err(|_| RmError::Other(crate::ABI_DECODE_FAILED))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MapPerm;

    /// ★ The raw issue path's memory-safety table (`kf_linux_raw::uvm::KERNEL_SIZES`) covers every
    /// op this session issues at least as large as the ABI's struct — the two statements of the
    /// same sizes can never drift apart silently.
    #[test]
    fn the_raw_bounds_cover_every_op() {
        for op in UvmOp::ALL {
            match op {
                UvmOp::EfsWait => {
                    assert_eq!(kf_linux_raw::uvm::EFS_WAIT_SIZE, op.kernel_size());
                }
                UvmOp::EfsResolve => {
                    assert_eq!(kf_linux_raw::uvm::EFS_RESOLVE_SIZE, op.kernel_size());
                }
                _ => assert_eq!(
                    kf_linux_raw::uvm::kernel_size(op.request()),
                    Some(op.kernel_size()),
                    "{}",
                    op.name()
                ),
            }
        }
        assert_eq!(kf_linux_raw::uvm::EFS_RECORD_SIZE, u::EFS_RECORD_SIZE);
        assert_eq!(
            kf_linux_raw::uvm::EFS_MAX_WAIT_RECORDS,
            u::UVM_EFS_MAX_WAIT_RECORDS as usize
        );
        assert_eq!(
            kf_linux_raw::uvm::EFS_MAX_RESOLVE_RECORDS,
            u::UVM_EFS_MAX_RESOLVE_RECORDS as usize
        );
    }

    /// The leaf's permissions become UVM's mapping and caching types, never wider.
    #[test]
    fn permissions_are_carried_never_widened() {
        let u = [1u8; 16];
        let rw = map_attrs(u, MapPerm::READ_WRITE);
        assert_eq!(rw.mapping, u::MappingType::ReadWriteAtomic);
        assert_eq!(rw.caching, u::CachingType::Default);
        let ro = map_attrs(
            u,
            MapPerm {
                read_only: true,
                atomic_disable: true,
                volatile: true,
            },
        );
        assert_eq!(ro.mapping, u::MappingType::ReadOnly, "read-only wins");
        assert_eq!(ro.caching, u::CachingType::ForceUncached);
        let na = map_attrs(
            u,
            MapPerm {
                read_only: false,
                atomic_disable: true,
                volatile: false,
            },
        );
        assert_eq!(na.mapping, u::MappingType::ReadWrite, "no atomics");
    }

    /// The refusals print what a reader needs to act on.
    #[test]
    fn the_refusals_name_the_host_state() {
        assert!(EfsRefusal::InitRefused(0x1f).to_string().contains("STOCK"));
        assert!(
            EfsRefusal::InitRefused(0x56)
                .to_string()
                .contains("uvm_efs_enable=0")
        );
        assert!(
            EfsRefusal::HostDriver("575.57.08".into())
                .to_string()
                .contains(EFS_HOST_DRIVER)
        );
    }
}
