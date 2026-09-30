//! ★★★ **`/dev/nvidia-uvm`, issued safely** — the memory-safety half of kf3's EFS mode
//! (`docs/design/V3_UVM_GUEST_FAULT_PLANE.md` §3.0).
//!
//! No `unsafe` here: every call goes through [`CharDevice::ioctl`]. What this file adds is the one
//! thing that call cannot know for this driver, and it is a memory-safety fact, so it lives in this
//! crate and nowhere else:
//!
//! ⊘ **nvidia-uvm's request numbers carry no size.** `UVM_IOCTL_BASE(i)` is `i`
//! (`ogkm-580: kernel-open/nvidia-uvm/uvm_ioctl.h`), so `_IOC_SIZE` decodes to 0 and the size guard
//! in [`CharDevice::ioctl`] says nothing — yet the driver copies **its own** `sizeof(params)` in
//! both directions (`uvm.c` `uvm_ioctl` → `UVM_ROUTE_CMD_*`). A buffer shorter than that is an
//! overrun with the kernel as the writer. ⇒ [`UvmFile::ioctl`] accepts only the requests in
//! [`KERNEL_SIZES`] and refuses a buffer shorter than the kernel's struct. And the famous exception,
//! `UVM_INITIALIZE = 0x30000001`, decodes to `_IOC_SIZE = 12288` against a 16-byte struct: the
//! guard would refuse it, so it is issued from a buffer of that decoded size (the driver copies its
//! 16 bytes and ignores the rest).
//!
//! ⊘ **Two EFS ioctls carry a user pointer**, and one of them WRITES through it:
//! `UVM_EFS_WAIT.records` receives up to `maxRecords × sizeof(UvmEfsFaultRecord)` bytes. So
//! [`UvmFile::efs_wait`] reads `maxRecords` out of the very argument it sends and refuses a record
//! buffer too small for it — the caller's buffer length is checked against the value the kernel
//! will use, never trusted. `UVM_EFS_RESOLVE.recordIds` is read (`count × 8` bytes) and checked
//! the same way, so the kernel never reads past what the caller owns.
//!
//! Sizes: `gcc offsetof/sizeof` over the 580.159.04 headers and the b3 patch's `uvm_efs_ioctl.h`
//! (ABI 1), 2026-09-30 — held equal to `kf_abi::uvmefs` by a test in `kf-host`.

use crate::chardev_unsafe::{CharDevice, DevDir, Indirect};
use crate::error::RawError;

/// `UVM_INITIALIZE` (`uvm_linux_ioctl.h:32`).
pub const UVM_INITIALIZE: u64 = 0x3000_0001;
/// What `_IOC_SIZE(UVM_INITIALIZE)` decodes to — the buffer the size guard demands for it.
pub const INITIALIZE_DECODED_SIZE: usize = 12_288;

/// `UVM_EFS_WAIT` and `UVM_EFS_RESOLVE` — the pointer-carrying pair ([`UvmFile::efs_wait`],
/// [`UvmFile::efs_resolve`]).
pub const UVM_EFS_WAIT: u64 = 1801;
/// See [`UVM_EFS_WAIT`].
pub const UVM_EFS_RESOLVE: u64 = 1802;
/// `sizeof(UVM_EFS_WAIT_PARAMS)`.
pub const EFS_WAIT_SIZE: usize = 24;
/// `sizeof(UVM_EFS_RESOLVE_PARAMS)`.
pub const EFS_RESOLVE_SIZE: usize = 32;
/// `sizeof(UvmEfsFaultRecord)`.
pub const EFS_RECORD_SIZE: usize = 88;
/// `UVM_EFS_MAX_WAIT_RECORDS`.
pub const EFS_MAX_WAIT_RECORDS: usize = 64;
/// `UVM_EFS_MAX_RESOLVE_RECORDS`.
pub const EFS_MAX_RESOLVE_RECORDS: usize = 256;

/// ★ `(request, sizeof the kernel's parameter struct)` for every flat (pointer-free) UVM request
/// kf3 issues. ⊘ A request not listed is refused: this table is the whole of what may be sent.
pub const KERNEL_SIZES: &[(u64, usize, &str)] = &[
    (UVM_INITIALIZE, 16, "UVM_INITIALIZE"),
    (25, 32, "UVM_REGISTER_GPU_VASPACE"),
    (26, 20, "UVM_UNREGISTER_GPU_VASPACE"),
    (27, 56, "UVM_REGISTER_CHANNEL"),
    (28, 28, "UVM_UNREGISTER_CHANNEL"),
    (33, 9264, "UVM_MAP_EXTERNAL_ALLOCATION"),
    (34, 24, "UVM_FREE"),
    (37, 40, "UVM_REGISTER_GPU"),
    (38, 20, "UVM_UNREGISTER_GPU"),
    (66, 40, "UVM_UNMAP_EXTERNAL"),
    (73, 24, "UVM_CREATE_EXTERNAL_RANGE"),
    (75, 8, "UVM_MM_INITIALIZE"),
    (1800, 168, "UVM_EFS_QUERY"),
];

/// The kernel's struct size for a flat request, or `None` for one this file does not issue.
#[must_use]
pub fn kernel_size(request: u64) -> Option<usize> {
    KERNEL_SIZES
        .iter()
        .find(|(r, _, _)| *r == request)
        .map(|&(_, s, _)| s)
}

/// An open `/dev/nvidia-uvm` file (one UVM va_space once initialized).
#[derive(Debug)]
pub struct UvmFile {
    dev: CharDevice,
}

fn u32_le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

impl UvmFile {
    /// Open `nvidia-uvm` relative to the granted `/dev`.
    ///
    /// # Errors
    /// The open's errno.
    pub fn open(dev: &DevDir) -> Result<UvmFile, RawError> {
        Ok(UvmFile {
            dev: CharDevice::openat(dev, c"nvidia-uvm")?,
        })
    }

    /// ★ Issue a flat request whose parameters are `arg` (exactly the kernel's struct, or longer).
    ///
    /// # Errors
    /// [`RawError::IoctlSizeMismatch`] — an unknown request, or `arg` shorter than the kernel's
    /// struct; otherwise the syscall's errno. The driver's own status is in `arg` (`rmStatus`).
    pub fn ioctl(&self, request: u64, arg: &mut [u8]) -> Result<i32, RawError> {
        let Some(need) = kernel_size(request) else {
            return Err(RawError::IoctlSizeMismatch {
                declared: 0,
                buffer: arg.len() as u64,
            });
        };
        if arg.len() < need {
            return Err(RawError::IoctlSizeMismatch {
                declared: need as u64,
                buffer: arg.len() as u64,
            });
        }
        if request == UVM_INITIALIZE {
            // The guard wants the decoded 12 288 bytes; the driver copies its 16.
            let mut big = vec![0u8; INITIALIZE_DECODED_SIZE];
            big[..need].copy_from_slice(&arg[..need]);
            let r = self.dev.ioctl(request, &mut big, &mut []);
            arg[..need].copy_from_slice(&big[..need]);
            return r;
        }
        self.dev.ioctl(request, arg, &mut [])
    }

    /// ★ `UVM_EFS_WAIT`: `arg` is the 24-byte params with `records` (offset 0) left zero; the kernel
    /// writes up to `maxRecords` records into `records`.
    ///
    /// # Errors
    /// [`RawError::OutOfRange`] — `maxRecords` above the ABI's 64, or a record buffer too small
    /// for `maxRecords` (checked against the value in `arg`, the one the kernel uses); otherwise
    /// the syscall's errno.
    pub fn efs_wait(&self, arg: &mut [u8], records: &mut [u8]) -> Result<i32, RawError> {
        if arg.len() < EFS_WAIT_SIZE {
            return Err(RawError::IoctlSizeMismatch {
                declared: EFS_WAIT_SIZE as u64,
                buffer: arg.len() as u64,
            });
        }
        let max = u32_le(arg, 8).unwrap_or(u32::MAX) as usize;
        let need = max.saturating_mul(EFS_RECORD_SIZE);
        if max > EFS_MAX_WAIT_RECORDS || records.len() < need {
            return Err(RawError::OutOfRange {
                offset: 0,
                len: need as u64,
                object_len: records.len() as u64,
            });
        }
        let mut patches = [Indirect::new(0, records)];
        self.dev.ioctl(UVM_EFS_WAIT, arg, &mut patches)
    }

    /// ★ `UVM_EFS_RESOLVE`: `arg` is the 32-byte params with `recordIds` (offset 0) left zero; the
    /// kernel reads `count` 8-byte ids from `ids`.
    ///
    /// # Errors
    /// [`RawError::OutOfRange`] — `count` above the ABI's 256 or an id buffer shorter than
    /// `count × 8`; otherwise the syscall's errno.
    pub fn efs_resolve(&self, arg: &mut [u8], ids: &mut [u8]) -> Result<i32, RawError> {
        if arg.len() < EFS_RESOLVE_SIZE {
            return Err(RawError::IoctlSizeMismatch {
                declared: EFS_RESOLVE_SIZE as u64,
                buffer: arg.len() as u64,
            });
        }
        let count = u32_le(arg, 8).unwrap_or(u32::MAX) as usize;
        let need = count.saturating_mul(8);
        if count > EFS_MAX_RESOLVE_RECORDS || ids.len() < need {
            return Err(RawError::OutOfRange {
                offset: 0,
                len: need as u64,
                object_len: ids.len() as u64,
            });
        }
        let mut patches = [Indirect::new(0, ids)];
        self.dev.ioctl(UVM_EFS_RESOLVE, arg, &mut patches)
    }

    /// The file descriptor number, for a log line. ⊘ Never a handle to act on.
    #[must_use]
    pub fn fd_number(&self) -> i32 {
        self.dev.fd_number()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every listed request is unique, no size is below the smallest UVM struct (16 bytes), and
    /// the pointer pair is NOT in the flat table (it must go through its own checked path).
    #[test]
    fn the_table_is_well_formed() {
        let mut seen = std::collections::BTreeSet::new();
        for &(r, s, n) in KERNEL_SIZES {
            assert!(seen.insert(r), "{n} listed twice");
            // `UVM_MM_INITIALIZE_PARAMS` (`{ NvS32 uvmFd; NV_STATUS rmStatus; }`) is the one
            // 8-byte struct; every other is at least 16.
            assert!(s >= 16 || (r == 75 && s == 8), "{n}");
        }
        assert_eq!(kernel_size(UVM_EFS_WAIT), None);
        assert_eq!(kernel_size(UVM_EFS_RESOLVE), None);
        assert_eq!(kernel_size(9999), None);
        assert_eq!(kernel_size(UVM_INITIALIZE), Some(16));
    }

    /// `UVM_INITIALIZE`'s decoded size is what the guard would demand.
    #[test]
    fn the_initialize_exception_is_what_the_guard_decodes() {
        assert_eq!(
            crate::ioctl::declared_size(UVM_INITIALIZE),
            INITIALIZE_DECODED_SIZE
        );
        for &(r, _, n) in KERNEL_SIZES {
            if r != UVM_INITIALIZE {
                assert_eq!(crate::ioctl::declared_size(r), 0, "{n} decodes to no size");
            }
        }
    }
}
