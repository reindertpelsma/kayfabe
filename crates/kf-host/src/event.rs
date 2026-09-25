//! ★★★ **Host events as a pollable fd — a WAKE, never a verdict, never an inline completion.**
//!
//! Owner ruling §37: *"with raw client you can obtain an eventfd and do the semaphore polling
//! yourself, keeping the big loop in the worker"*. The worker's only waits are its work mutex
//! and `epoll` (§35); a host completion reaches it as **readiness on this fd**, and the worker
//! then reads the SEMAPHORE — the only source of truth.
//!
//! The protocol is CUDA's own, read off the host trace (`traces/host_reference_ga106/ce_r1`,
//! records 260-262) and the OS layer (ogkm-580 `osapi.c:2782-2820`):
//! 1. open a fresh GPU node `nvidia<N>` and `REGISTER_FD` it against the control fd;
//! 2. `NV_ESC_ALLOC_OS_EVENT` **on that file** with `{hClient, hDevice, fd}` — `fd` is only a KEY
//!    (`allocate_os_event`, `osapi.c:505`);
//! 3. `RM_ALLOC NV01_EVENT_OS_EVENT` **on that same file**, `data = the key`;
//! 4. arm the notifier (`EVENT_SET_NOTIFICATION`, action REPEAT) on the subdevice;
//! 5. `poll`/`epoll` the file (`nvidia_poll`, `nv.c:2287-2296`).
//!
//! ★ **Every event is registered DATALESS** (`NV01_EVENT_WITHOUT_EVENT_DATA`,
//! `event_notification.c:809`). A data event makes `nv_post_event` `KMALLOC_ATOMIC` one record per
//! interrupt per listener with no cap (`nv.c:3997-4015`); a dataless one sets ONE flag that
//! `nvidia_poll` reports and clears (`nv.c:4026`, `:2292-2295`) — a coalesced, bounded wake. The
//! records carried nothing we could use anyway: the non-stall notifiers are GPU-wide and
//! `osNotifyEvent` posts `info32 = info16 = 0`, so a record cannot say WHOSE work finished.
//! ⊘ Hence no drain verb: readiness says "look", the semaphore says "done".

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
use kf_linux_raw::{CharDevice, ioctl};
use std::os::fd::BorrowedFd;

/// `NV_ESC_ALLOC_OS_EVENT` = `NV_IOCTL_BASE + 6` (`ogkm-580: nv-ioctl-numbers.h:33`).
const NV_ESC_ALLOC_OS_EVENT: u8 = 206;
/// `sizeof(nv_ioctl_alloc_os_event_t)` — `{hClient, hDevice, fd, Status}`.
const ALLOC_OS_EVENT_SIZE: usize = 16;
/// `sizeof(NV0005_ALLOC_PARAMETERS)` — `{hParentClient, hSrcResource, hClass, notifyIndex,
/// NvP64 data}` (`ogkm-580: class/cl0005.h:39-47`).
const NV0005_PARAMS_SIZE: usize = 24;
/// `NV01_EVENT_NONSTALL_INTR` (`ogkm-580: nvos.h:433`), OR-ed into `notifyIndex`. ★ Without it an
/// engine event lands on the subdevice's ORDINARY notifier list, which a CE non-stall interrupt never
/// walks: `engineNonStallIntrNotify` notifies only `pGpu->engineNonstallIntrEventNotifications`,
/// and an event joins that list only when this bit is set (`event_notification.c:683-737`).
/// `[measured w826 gate 1]` without it: copy + semaphore done, event fd silent on all ten CEs.
pub const NV01_EVENT_NONSTALL_INTR: u32 = 0x0800_0000;

/// `NV01_EVENT_WITHOUT_EVENT_DATA` (`ogkm-580: nvos.h:430`), OR-ed into `notifyIndex` on EVERY
/// registration — see the module doc.
pub const NV01_EVENT_WITHOUT_EVENT_DATA: u32 = 0x1000_0000;

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

/// `NV2080_NOTIFIERS_NVENC(x)` — `NVENC0..2` = 38..40, `NVENC3` = 183
/// (`ogkm-580: class/cl2080_notification.h:75-78,224,253`).
#[must_use]
pub const fn notifier_nvenc(n: u32) -> u32 {
    if n < 3 { 38 + n } else { 183 + n - 3 }
}

/// `NV2080_NOTIFIERS_NVDEC(x)` — `NVDEC0` (= `_VLD`) = 14, then contiguous
/// (`cl2080_notification.h:50-58,258`).
#[must_use]
pub const fn notifier_nvdec(n: u32) -> u32 {
    14 + n
}

/// ★ A pollable host event source: register [`EventFd::as_fd`] with the worker's `epoll`.
/// Readiness is a coalesced WAKE; the caller must read its semaphore to learn what completed.
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
}

impl HostRm {
    /// Open a fresh GPU node and register it as an OS-event target for our client.
    ///
    /// # Errors
    /// The open, or the host's refusal of the registration.
    pub fn open_event_fd(&self) -> Result<EventFd, RmError> {
        // ★ CUDA's own sequence, read off the host trace (`traces/host_reference_ga106/ce_r1`,
        // records 260-262): a fresh GPU node, `REGISTER_FD` against the control fd, then
        // `ALLOC_OS_EVENT` on it keyed by its own fd number — and the event object's `RM_ALLOC`
        // is issued ON THAT SAME FILE. `[measured w826 gate 1]` a `nvidiactl` event file with the
        // alloc on the main control file posted nothing on any CE notifier.
        let name = std::ffi::CString::new(format!("nvidia{}", self.gpu_index))
            .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        let node = CharDevice::openat(&self.dev, &name).map_err(|e| ioctl_error(&e))?;
        let mut reg = [0u8; 4];
        kf_abi::bringup::RegisterFd { ctl_fd: self.ctl.fd_number() }
            .encode_into(&mut reg)
            .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        let req = ioctl::readwrite(NV_IOCTL_MAGIC, kf_abi::bringup::NV_ESC_REGISTER_FD, reg.len())
            .map_err(|_| RmError::Other(IOCTL_NUMBER_UNBUILDABLE))?;
        node.ioctl(req, &mut reg, &mut []).map_err(|e| ioctl_error(&e))?;
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

    /// A dataless `NV01_EVENT_OS_EVENT` under `source` for `notify_index`, delivered to `ev`.
    /// `nonstall` registers it on the ENGINE's non-stall list (source must be the subdevice).
    /// ⚠ The object lives until the session's client is freed: the event file closing only marks
    /// it inactive (`free_os_events`), so allocate these once per session, never per submit.
    ///
    /// # Errors
    /// The host's refusal.
    pub fn alloc_os_event(
        &self,
        source: u32,
        notify_index: u32,
        nonstall: bool,
        ev: &EventFd,
    ) -> Result<u32, RmError> {
        let notify_index = notify_index
            | NV01_EVENT_WITHOUT_EVENT_DATA
            | if nonstall { NV01_EVENT_NONSTALL_INTR } else { 0 };
        let mut params = [0u8; NV0005_PARAMS_SIZE];
        params[0..4].copy_from_slice(&self.client.raw().to_le_bytes());
        params[4..8].copy_from_slice(&source.to_le_bytes());
        params[8..12].copy_from_slice(&NV01_EVENT_OS_EVENT.to_le_bytes());
        params[12..16].copy_from_slice(&notify_index.to_le_bytes());
        params[16..24].copy_from_slice(&u64::from(ev.key).to_le_bytes());
        let want = self.mint();
        let h = self.raw_alloc_via(&ev.node, source, want, NV01_EVENT_OS_EVENT, &mut params)?;
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

    /// ★ Arm `notify_index` REPEAT for the session, once. Idempotent: a second caller (another
    /// channel's ring) is a no-op, never the `NV_ERR_INVALID_STATE` a re-arm earns
    /// (`subdevice_ctrl_event_kernel.c:123-130` — `[measured w826 gate 4]` `0x40` on the second).
    ///
    /// # Errors
    /// The host's status on the first arm.
    pub fn arm_repeat(&self, notify_index: u32) -> Result<(), RmError> {
        let mut armed = self.armed.lock().map_err(|_| RmError::Other(crate::NOT_ON_THIS_RUNG))?;
        if armed.contains(&notify_index) {
            return Ok(());
        }
        self.set_notification(notify_index, kf_abi::eventnotify::ACTION_REPEAT)?;
        armed.insert(notify_index);
        Ok(())
    }

    /// The session's device handle (`NV01_DEVICE_0` — the object `NV0080` controls address).
    #[must_use]
    pub fn device(&self) -> u32 {
        self.device
    }

    /// The session's subdevice handle (the parent of engine notifiers).
    #[must_use]
    pub fn subdevice(&self) -> u32 {
        self.subdevice
    }
}
