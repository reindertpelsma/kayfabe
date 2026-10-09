// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ The three kernel-uapi verbs of the GPU-copy broker rung that hand back or name a descriptor
//! (`docs/design/V3_DISPLAY.md` §8.11): `DRM_IOCTL_PRIME_HANDLE_TO_FD` (a GEM handle becomes a
//! dma-buf), `DRM_IOCTL_GEM_CLOSE` (a GEM handle that was never sent goes), and
//! `DMA_BUF_IOCTL_EXPORT_SYNC_FILE` + `poll(0)` (is anyone still reading a dma-buf kayfabe wants
//! to rewrite?). Each struct is the kernel's own (`linux: include/uapi/drm/drm.h`,
//! `include/uapi/linux/dma-buf.h`), mirrored `#[repr(C)]` with its size asserted, and each
//! descriptor the kernel returns is adopted on the line that received it.
//!
//! ⊘ None of these blocks: `PRIME_HANDLE_TO_FD` and `GEM_CLOSE` take DRM's object locks (never
//! RM's), `EXPORT_SYNC_FILE` iterates the reservation's fences WITHOUT its lock
//! (`linux: drivers/dma-buf/dma-resv.c:579`, via `dma_resv_get_singleton`) — which is why it is
//! used rather than `poll()` on the dma-buf itself, which takes `dma_resv_lock`
//! (`dma-buf.c:355`) — and the `poll` has a zero timeout.

use crate::RawError;
use crate::drm::DrmRender;
use crate::error::last_syscall_error;
use crate::host_fd_unsafe::adopt_fd;
use kf_util::{leafwitness, lockwitness};
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};

/// `DRM_IOCTL_PRIME_HANDLE_TO_FD` = `DRM_IOWR(0x2d, struct drm_prime_handle)` (`drm.h`).
pub const PRIME_HANDLE_TO_FD: u32 = 0xC00C_642D;
/// `DRM_IOCTL_GEM_CLOSE` = `DRM_IOW(0x09, struct drm_gem_close)` (`drm.h`).
pub const GEM_CLOSE: u32 = 0x4008_6409;
/// `DMA_BUF_IOCTL_EXPORT_SYNC_FILE` = `_IOWR('b', 2, struct dma_buf_export_sync_file)`
/// (`linux: include/uapi/linux/dma-buf.h`).
pub const DMA_BUF_EXPORT_SYNC_FILE: u32 = 0xC008_6202;
/// `DMA_BUF_SYNC_WRITE` (`dma-buf.h`): the fence a WRITER must wait for — every reader's and
/// every writer's.
pub const DMA_BUF_SYNC_WRITE: u32 = 2;
/// `DRM_RDWR` (= `O_RDWR`, `drm.h`).
const DRM_RDWR: u32 = libc::O_RDWR as u32;
/// `DRM_CLOEXEC` (= `O_CLOEXEC`, `drm.h`).
const DRM_CLOEXEC: u32 = libc::O_CLOEXEC as u32;

/// `struct drm_prime_handle` (`drm.h`).
#[repr(C)]
struct DrmPrimeHandle {
    handle: u32,
    flags: u32,
    fd: i32,
}
const _: () = assert!(core::mem::size_of::<DrmPrimeHandle>() == 12);

/// `struct drm_gem_close` (`drm.h`).
#[repr(C)]
struct DrmGemClose {
    handle: u32,
    pad: u32,
}
const _: () = assert!(core::mem::size_of::<DrmGemClose>() == 8);

/// `struct dma_buf_export_sync_file` (`dma-buf.h`).
#[repr(C)]
struct DmaBufExportSyncFile {
    flags: u32,
    fd: i32,
}
const _: () = assert!(core::mem::size_of::<DmaBufExportSyncFile>() == 8);

/// ★ GEM handle `handle` on `node` as a dma-buf descriptor (close-on-exec, read-write). ⊘ PRIME
/// caches one dma-buf per GEM object (`linux: drivers/gpu/drm/drm_prime.c:446-466`): a second
/// call returns the SAME file, so a caller that needs a fresh identity must import a fresh GEM
/// object.
///
/// # Errors
/// [`RawError::Syscall`].
pub fn prime_handle_to_fd(node: &DrmRender, handle: u32) -> Result<OwnedFd, RawError> {
    lockwitness::assert_lock_free("ioctl(DRM_IOCTL_PRIME_HANDLE_TO_FD)");
    leafwitness::assert_leaf_free("ioctl(DRM_IOCTL_PRIME_HANDLE_TO_FD)");
    let mut arg = DrmPrimeHandle {
        handle,
        flags: DRM_CLOEXEC | DRM_RDWR,
        fd: -1,
    };
    // SAFETY: `arg` is a live `#[repr(C)]` mirror of `struct drm_prime_handle` (12 bytes,
    // asserted at compile time) in this frame, and the request encodes that same size; the kernel
    // reads and writes exactly that struct and keeps no pointer to it. `node` is live for the call.
    let rc = unsafe {
        libc::ioctl(
            node.as_fd().as_raw_fd(),
            PRIME_HANDLE_TO_FD as _,
            core::ptr::from_mut(&mut arg),
        )
    };
    if rc < 0 {
        return Err(last_syscall_error("ioctl(DRM_IOCTL_PRIME_HANDLE_TO_FD)"));
    }
    // the kernel installed a NEW descriptor in `arg.fd` (`drm_prime_handle_to_fd_ioctl`): this is
    // its only owner
    adopt_fd(arg.fd, "ioctl(DRM_IOCTL_PRIME_HANDLE_TO_FD)")
}

/// GEM handle `handle` on `node` goes (a colliding import that was never sent). The dma-buf a
/// previous [`prime_handle_to_fd`] made keeps the object alive on its own.
///
/// # Errors
/// [`RawError::Syscall`].
pub fn gem_close(node: &DrmRender, handle: u32) -> Result<(), RawError> {
    lockwitness::assert_lock_free("ioctl(DRM_IOCTL_GEM_CLOSE)");
    leafwitness::assert_leaf_free("ioctl(DRM_IOCTL_GEM_CLOSE)");
    let mut arg = DrmGemClose { handle, pad: 0 };
    // SAFETY: `arg` is a live `#[repr(C)]` mirror of `struct drm_gem_close` (8 bytes, asserted)
    // and the request encodes that size; `_IOW`, the kernel only reads it. `node` is live.
    let rc = unsafe {
        libc::ioctl(
            node.as_fd().as_raw_fd(),
            GEM_CLOSE as _,
            core::ptr::from_mut(&mut arg),
        )
    };
    if rc < 0 {
        return Err(last_syscall_error("ioctl(DRM_IOCTL_GEM_CLOSE)"));
    }
    Ok(())
}

/// ★ Has every fence on dma-buf `dmabuf` signalled — may kayfabe rewrite it without tearing a
/// frame a compositor's GPU is still reading? `EXPORT_SYNC_FILE(WRITE)` collects every fence
/// into one sync_file WITHOUT the reservation lock; a zero-timeout `poll` asks whether it has
/// signalled; the sync_file is closed. Never waits.
///
/// ⚠ A compositor that attaches NO fence (X11 presents with `idle_fence = None`) makes this say
/// "idle" while its GPU may still read: this check narrows the window, it cannot close it
/// (§8.11 — the LRU fill is the other half).
///
/// # Errors
/// [`RawError::Syscall`] — `ENOTTY` on a kernel before 6.0 (no `EXPORT_SYNC_FILE`), or on a
/// descriptor that is not a dma-buf.
pub fn dma_buf_idle(dmabuf: BorrowedFd<'_>) -> Result<bool, RawError> {
    lockwitness::assert_lock_free("ioctl(DMA_BUF_IOCTL_EXPORT_SYNC_FILE)");
    leafwitness::assert_leaf_free("ioctl(DMA_BUF_IOCTL_EXPORT_SYNC_FILE)");
    let mut arg = DmaBufExportSyncFile {
        flags: DMA_BUF_SYNC_WRITE,
        fd: -1,
    };
    // SAFETY: `arg` is a live `#[repr(C)]` mirror of `struct dma_buf_export_sync_file` (8 bytes,
    // asserted) and the request encodes that size; the kernel reads `flags` and writes `fd`, and
    // keeps no pointer. `dmabuf` is live for the call.
    let rc = unsafe {
        libc::ioctl(
            dmabuf.as_raw_fd(),
            DMA_BUF_EXPORT_SYNC_FILE as _,
            core::ptr::from_mut(&mut arg),
        )
    };
    if rc < 0 {
        return Err(last_syscall_error("ioctl(DMA_BUF_IOCTL_EXPORT_SYNC_FILE)"));
    }
    let sync = adopt_fd(arg.fd, "ioctl(DMA_BUF_IOCTL_EXPORT_SYNC_FILE)")?;
    let mut p = libc::pollfd {
        fd: sync.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: `p` is one live `pollfd` in this frame and the count says one; the timeout is zero,
    // so the call never blocks. `sync` stays open until after the call (dropped below).
    let n = unsafe { libc::poll(&raw mut p, 1, 0) };
    if n < 0 {
        return Err(last_syscall_error("poll(sync_file)"));
    }
    Ok(n == 1 && p.revents & libc::POLLIN != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SharedRam;
    use std::os::fd::AsFd as _;

    /// The request numbers are the `_IOC` encodings of the mirrored structs.
    #[test]
    fn the_uapi_numbers_encode_their_structs() {
        let ioc = |dir: u32, ty: u8, nr: u32, size: usize| {
            (dir << 30) | ((size as u32) << 16) | (u32::from(ty) << 8) | nr
        };
        assert_eq!(
            PRIME_HANDLE_TO_FD,
            ioc(3, b'd', 0x2d, core::mem::size_of::<DrmPrimeHandle>())
        );
        assert_eq!(
            GEM_CLOSE,
            ioc(1, b'd', 0x09, core::mem::size_of::<DrmGemClose>())
        );
        assert_eq!(
            DMA_BUF_EXPORT_SYNC_FILE,
            ioc(3, b'b', 2, core::mem::size_of::<DmaBufExportSyncFile>())
        );
    }

    /// ★ `/dev/udmabuf` gated: a dma-buf nobody has fenced is idle; a memfd (not a dma-buf) is
    /// refused by the kernel, never read as "idle".
    #[test]
    fn an_unfenced_dma_buf_is_idle_and_a_memfd_is_refused() {
        crate::require_udmabuf!("an_unfenced_dma_buf_is_idle_and_a_memfd_is_refused");
        let dev = crate::udmabuf_gate::open_device().expect("the gate opened it");
        let page = crate::HostPageSize::query();
        let ram = SharedRam::create_named(c"kfu-fence-test", page.bytes()).expect("memfd");
        let buf = crate::udmabuf_create(dev.as_fd(), &ram, page).expect("UDMABUF_CREATE");
        match dma_buf_idle(buf.as_fd()) {
            Ok(idle) => assert!(idle, "no fence was ever attached"),
            // a kernel before 6.0 has no EXPORT_SYNC_FILE: say so, never call it idle
            Err(RawError::Syscall {
                errno: Some(libc::ENOTTY),
                ..
            }) => kf_util::klog!("EXPORT_SYNC_FILE: ENOTTY on this kernel (< 6.0)"),
            Err(e) => panic!("{e}"),
        }
        assert!(dma_buf_idle(ram.as_backing_fd()).is_err());
    }
}
