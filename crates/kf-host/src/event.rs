//! ★★★ **Host events as a pollable fd — the completion edge, never an inline completion.**
//!
//! Owner ruling §37: *"with raw client you can obtain an eventfd and do the semaphore polling
//! yourself, keeping the big loop in the worker"*. The worker's only waits are its work mutex
//! and `epoll` (§35); a host completion reaches it as **readiness on this fd**.
//!
//! The protocol is the OS layer's own (ogkm-580 `osapi.c:2782-2820`, never reaching RM core):
//! 1. open a dedicated `/dev/nvidiactl` — events bind to the file the ioctl is issued on;
//! 2. `NV_ESC_ALLOC_OS_EVENT` **on that file** with `{hClient, hDevice, fd}` — `fd` is only a KEY
//!    (`allocate_os_event`, `osapi.c:505`), one event per `(hClient, fd)`;
//! 3. `RM_ALLOC NV01_EVENT_OS_EVENT` under the source object with `data = the same key`;
//! 4. arm the notifier (`EVENT_SET_NOTIFICATION`, action REPEAT) on the subdevice;
//! 5. `poll` the file (`nvidia_fops.poll`, `nv.c:238`), then drain with
//!    `NV_ESC_RM_GET_EVENT_DATA` until `MoreEvents == 0` (`nv_get_event`, `nv.c:4149`).
//!
//! ⚠ `[not yet measured]` whether our own CE channel's non-stall interrupt reaches the
//! `NV2080_NOTIFIERS_CE(n)` event at the granularity the planes need. The harness measures it;
//! until then a semaphore POLL stays legal (§35), with this fd as the wake.

use crate::{
    ABI_DECODE_FAILED, ABI_ENCODE_FAILED, HostRm, IOCTL_NUMBER_UNBUILDABLE, RmError, ioctl_error,
    status_check,
};
use kf_abi::bringup::NV_IOCTL_MAGIC;
use kf_abi::eventnotify::{
    ACTION_OFF, EVENT_OFF, EVENT_SET_NOTIFICATION_PARAMS_SIZE, NOTIFY_STATE_OFF,
    NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION,
};
use kf_abi::generated::classes::NV01_EVENT_OS_EVENT;
use kf_linux_raw::{CharDevice, Indirect, ioctl};
use std::os::fd::BorrowedFd;

/// `NV_ESC_ALLOC_OS_EVENT` = `NV_IOCTL_BASE + 6` (`ogkm-580: nv-ioctl-numbers.h:33`).
const NV_ESC_ALLOC_OS_EVENT: u8 = 206;
/// `NV_ESC_RM_GET_EVENT_DATA` (`ogkm-580: nv_escape.h:44`).
const NV_ESC_RM_GET_EVENT_DATA: u8 = 0x52;
/// `sizeof(nv_ioctl_alloc_os_event_t)` — `{hClient, hDevice, fd, Status}`.
const ALLOC_OS_EVENT_SIZE: usize = 16;
/// `sizeof(NV0005_ALLOC_PARAMETERS)` — `{hParentClient, hSrcResource, hClass, notifyIndex,
/// NvP64 data}` (`ogkm-580: class/cl0005.h:39-47`).
const NV0005_PARAMS_SIZE: usize = 24;
/// `sizeof(NVOS41_PARAMETERS)` — `{NvP64 pEvent, MoreEvents, status}` (`nvos.h:1940-1945`).
const NVOS41_SIZE: usize = 16;
/// `sizeof(NvUnixEvent)` — `{hObject, NotifyIndex, info32, NvU16 info16}` + 2 pad
/// (`nvos.h:1926-1937`).
const NV_UNIX_EVENT_SIZE: usize = 16;
/// Drain bound per readiness — the queue is RM's; we never loop on its word alone.
pub const DRAIN_MAX: usize = 64;

/// `NV2080_NOTIFIERS_CE0` (`ogkm-580: class/cl2080_notification.h:60`).
pub const NV2080_NOTIFIERS_CE0: u32 = 23;
/// `NV2080_NOTIFIERS_CE10` (`cl2080_notification.h`).
pub const NV2080_NOTIFIERS_CE10: u32 = 184;

/// The subdevice notifier index for copy engine `n` (`NV2080_NOTIFIERS_CE(x)`,
/// `cl2080_notification.h:247`).
#[must_use]
pub const fn notifier_ce(n: u32) -> u32 {
    if n < 10 { NV2080_NOTIFIERS_CE0 + n } else { NV2080_NOTIFIERS_CE10 + n - 10 }
}

/// One drained event record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostEvent {
    /// The event object RM posted for.
    pub object: u32,
    /// Its notifier index.
    pub notify_index: u32,
    /// `NvNotification::info32`.
    pub info32: u32,
    /// `NvNotification::info16`.
    pub info16: u16,
}

/// ★ A pollable host event source: register [`EventFd::as_fd`] with the worker's `epoll`.
#[derive(Debug)]
pub struct EventFd {
    node: CharDevice,
    key: u32,
}

impl EventFd {
    /// The descriptor to watch for readability.
    #[must_use]
    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.node.as_fd()
    }

    /// The key events are bound under (`data` of every `NV01_EVENT_OS_EVENT` on this fd).
    #[must_use]
    pub fn key(&self) -> u32 {
        self.key
    }

    /// Drain up to [`DRAIN_MAX`] pending records (`NV_ESC_RM_GET_EVENT_DATA`). An empty queue is
    /// `Ok(empty)`: RM answers `NV_ERR_GENERIC` for "nothing pending" (`nv_get_event`).
    ///
    /// # Errors
    /// An ioctl-level failure.
    pub fn drain(&self) -> Result<Vec<HostEvent>, RmError> {
        let mut out = Vec::new();
        for _ in 0..DRAIN_MAX {
            let mut rec = [0u8; NV_UNIX_EVENT_SIZE];
            let mut arg = [0u8; NVOS41_SIZE];
            let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_RM_GET_EVENT_DATA, arg.len())
                .map_err(|_| RmError::Other(IOCTL_NUMBER_UNBUILDABLE))?;
            let mut patches = [Indirect::new(0, &mut rec)];
            self.node
                .ioctl(req, &mut arg, &mut patches)
                .map_err(|e| ioctl_error(&e))?;
            let more = u32::from_le_bytes(arg[8..12].try_into().unwrap_or([0; 4]));
            let status = u32::from_le_bytes(arg[12..16].try_into().unwrap_or([0; 4]));
            if status != 0 {
                break; // nothing pending
            }
            let w = |o: usize| u32::from_le_bytes(rec[o..o + 4].try_into().unwrap_or([0; 4]));
            out.push(HostEvent {
                object: w(0),
                notify_index: w(4),
                info32: w(8),
                info16: u16::from_le_bytes([rec[12], rec[13]]),
            });
            if more == 0 {
                break;
            }
        }
        Ok(out)
    }
}

impl HostRm {
    /// Open a dedicated `/dev/nvidiactl` and register it as an OS-event target for our client.
    ///
    /// # Errors
    /// The open, or the host's refusal of the registration.
    pub fn open_event_fd(&self) -> Result<EventFd, RmError> {
        let node = CharDevice::openat(&self.dev, c"nvidiactl").map_err(|e| ioctl_error(&e))?;
        let key = u32::try_from(node.fd_number()).map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        let mut arg = [0u8; ALLOC_OS_EVENT_SIZE];
        arg[0..4].copy_from_slice(&self.client.raw().to_le_bytes());
        arg[4..8].copy_from_slice(&self.device.to_le_bytes());
        arg[8..12].copy_from_slice(&key.to_le_bytes());
        let req = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_ALLOC_OS_EVENT, arg.len())
            .map_err(|_| RmError::Other(IOCTL_NUMBER_UNBUILDABLE))?;
        node.ioctl(req, &mut arg, &mut []).map_err(|e| ioctl_error(&e))?;
        status_check(u32::from_le_bytes(arg[12..16].try_into().map_err(|_| RmError::Other(ABI_DECODE_FAILED))?))?;
        Ok(EventFd { node, key })
    }

    /// `NV01_EVENT_OS_EVENT` under `source` for `notify_index`, delivered to `ev`.
    ///
    /// # Errors
    /// The host's refusal.
    pub fn alloc_os_event(&self, source: u32, notify_index: u32, ev: &EventFd) -> Result<u32, RmError> {
        let mut params = [0u8; NV0005_PARAMS_SIZE];
        params[0..4].copy_from_slice(&self.client.raw().to_le_bytes());
        params[4..8].copy_from_slice(&source.to_le_bytes());
        params[8..12].copy_from_slice(&NV01_EVENT_OS_EVENT.to_le_bytes());
        params[12..16].copy_from_slice(&notify_index.to_le_bytes());
        params[16..24].copy_from_slice(&u64::from(ev.key).to_le_bytes());
        let want = self.mint();
        let h = self.raw_alloc(source, want, NV01_EVENT_OS_EVENT, &mut params)?;
        self.remember(h, source);
        Ok(h)
    }

    /// Arm `notify_index` on the subdevice with `action` (`ACTION_REPEAT` for a standing edge).
    ///
    /// # Errors
    /// The host's status.
    pub fn set_notification(&self, notify_index: u32, action: u32) -> Result<(), RmError> {
        let mut p = [0u8; EVENT_SET_NOTIFICATION_PARAMS_SIZE];
        p[EVENT_OFF..EVENT_OFF + 4].copy_from_slice(&notify_index.to_le_bytes());
        p[ACTION_OFF..ACTION_OFF + 4].copy_from_slice(&action.to_le_bytes());
        p[NOTIFY_STATE_OFF] = 0;
        self.raw_control(self.subdevice, NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION, &mut p)
    }

    /// The session's subdevice handle (the parent of engine notifiers).
    #[must_use]
    pub fn subdevice(&self) -> u32 {
        self.subdevice
    }
}
