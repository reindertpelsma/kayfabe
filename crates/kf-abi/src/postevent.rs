//! ★★★ The **os-event wakeup** a GSP sends: `NV_VGPU_MSG_EVENT_POST_EVENT` (`0x1003`).
//!
//! This module owns exactly one thing — the bytes of `rpc_post_event_v17_00` — and it owns
//! them for [`crate::rc`]'s reason exactly (decision #2, the quarantine): the layout is an
//! NVIDIA `#[repr(C)]`, so no crate above may state a field offset for it.
//!
//! # What this message IS, in the guest's own code
//!
//! `_kgspRpcPostEvent` (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:497-535`,
//! `ogkm-610: :502-540`) reads the body, and with `bNotifyList == NV_FALSE` it does:
//!
//! ```text
//! CliGetEventInfo(hClient, hEvent, &pEventInfo)   /* the (client, event) MATCH */
//! osNotifyEvent(pGpu, pEventInfo, 0, data, status)
//! ```
//!
//! On Linux `osNotifyEvent` reaches `nv_post_event`, which signals the file descriptor a
//! userspace waiter is blocked in `poll()` on. ⇒ **this message is the wakeup**, and the
//! pair it is matched on is `(hClient, hEvent)` — precisely the pair
//! `kayfabe_device::osevent` records off the `NV01_EVENT_OS_EVENT` alloc.
//!
//! # ⊘ The event data is EMPTY, and that is a decision with the same shape as `rc`'s
//!
//! `eventDataSize = 0` and `bNotifyList = NV_FALSE`. The list form
//! (`bNotifyList = NV_TRUE`) walks `pEventInfo` chains and copies `eventData[]` into
//! per-notifier buffers; we have no event data — nothing was read off any silicon — and
//! inventing a payload would put fabricated bytes into a guest notifier. An empty,
//! non-list POST_EVENT says exactly *"the event you registered has fired"*, which is the
//! whole of what a blocking-sync waiter needs to re-check its own semaphore.
//!
//! ★ `C: src/qemu/nvkvm_gpu_emul.c:1806-1821` makes the identical choice, field for
//! field, and its `rpc.length` is `32 + 32` — the 32-byte envelope plus exactly
//! [`RpcPostEventV1700::SIZE`]. That is the only implementation a real NVIDIA driver has
//! ever accepted end to end.

use crate::generated::rpc::RpcPostEventV1700;

/// One `POST_EVENT`, in field terms.
///
/// ⊘ There is deliberately no `Default`: a zeroed post event names `hClient = 0`,
/// `hEvent = 0` — a pair `CliGetEventInfo` cannot resolve — which is a well-formed message
/// about nothing, and the guest answers it by desynchronising nothing and doing nothing.
/// Every field here has to come from a registration that was actually observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostEvent {
    /// `hClient` — the namespace the event was registered in.
    pub client: u32,
    /// `hEvent` — the event object's handle, as the guest's `GSP_RM_ALLOC` named it.
    pub event: u32,
    /// `notifyIndex` — echoed back exactly as registered.
    ///
    /// ⚠ Echoed, never interpreted. It carries `NV01_EVENT_CLIENT_RM` in its top bits on
    /// this path (`ogkm-580: rmapi/event.c:161-171` sets `notifyIndex | NV01_EVENT_CLIENT_RM`
    /// when it builds the RPC's stack-local params), and the guest's own
    /// `CliGetEventInfo`/`osNotifyEvent` pair is what gives the number meaning. A port that
    /// "normalised" it would be answering a question it was not asked.
    pub notify_index: u32,
}

impl PostEvent {
    /// `sizeof(rpc_post_event_v17_00)` with an empty `eventData[]`.
    pub const SIZE: usize = RpcPostEventV1700::SIZE;

    /// The encoded body, as the form the RPC transport wants.
    ///
    /// Every field the struct declares is written — including the five that are zero — so
    /// the image never depends on what a caller's buffer happened to hold.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = vec![0u8; Self::SIZE];
        let m = RpcPostEventV1700 {
            h_client: self.client,
            h_event: self.event,
            notify_index: self.notify_index,
            // ⊘ `data` is the one word `osNotifyEvent` hands the waiter, and zero is the
            // honest value: this port has no per-event datum to report. The C sends zero
            // too (`C:1866`, the last argument of every `nvkvm_m3_post_event` call).
            data: 0,
            info16: 0,
            // `status` here is the EVENT's status, not the RPC's — `osNotifyEvent`'s
            // `status` argument. `NV_OK`.
            status: 0,
            // See the module docs: an empty, non-list event.
            event_data_size: 0,
            b_notify_list: 0,
        };
        // Infallible: the buffer is exactly `SIZE`, which is `encode_into`'s only failure.
        m.encode_into(&mut buf).expect("buffer is exactly SIZE");
        buf
    }
}

kf_util::assert_send_sync!(PostEvent);

/// `NV2080_NOTIFIERS_HOTPLUG` (`ogkm-580: src/common/sdk/nvidia/inc/class/cl2080_notification.h:37`).
pub const NOTIFIERS_HOTPLUG: u32 = 1;
/// `NV2080_NOTIFIERS_MAXCOUNT` (`cl2080_notification.h:239`): `gpuNotifySubDeviceEvent` asserts a
/// list post's `notifyIndex` is below it and indexes `notifyActions[]` with it
/// (`ogkm-580: src/nvidia/src/kernel/gpu/gpu_rmapi.c:532`, `:571`).
pub const NOTIFIERS_MAXCOUNT: u32 = 198;
/// Where `eventData[]` starts: it is a flexible `NvU8` array right after `NvBool bNotifyList` @ +28
/// (`g_rpc-structures.h:1545-1556`), so at **+29**, inside the padding `sizeof` rounds up to 32.
pub const EVENT_DATA_AT: usize = 29;

/// ★ Display step 3c: a LIST post (`bNotifyList = NV_TRUE`). `_kgspRpcPostEvent` resolves the
/// `(hClient, hEvent)` pair, then calls `gpuNotifySubDeviceEvent(notifyIndex, eventData, …)`, which
/// wakes every subdevice notifier the guest's own RM has armed for that index
/// (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:497-535`) — so the guest RM, not this
/// device, decides whom to wake.
///
/// ⊘ The index is the BARE notifier (`NV2080_NOTIFIERS_HOTPLUG` = 1), never the registered
/// `idx | NV01_EVENT_CLIENT_RM` that [`PostEvent`] echoes: on the list path it is an array index
/// in the guest kernel, and [`SubdeviceNotify::encode`] refuses anything at or above
/// [`NOTIFIERS_MAXCOUNT`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubdeviceNotify {
    /// `hClient` of a live registration.
    pub client: u32,
    /// `hEvent` of that registration (`CliGetEventInfo` must resolve the pair).
    pub event: u32,
    /// The bare `NV2080_NOTIFIERS_*` index.
    pub notify_index: u32,
    /// `eventData`: for a hotplug, `Nv2080HotplugNotification { plugDisplayMask,
    /// unplugDisplayMask }` (`cl2080_notification.h:485-489`).
    pub event_data: [u8; 8],
}

impl SubdeviceNotify {
    /// The hotplug post the guest's own fake-plug path builds the same way
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/disp/disp_common_kern_ctrl_minimal.c:262-278`):
    /// `plugDisplayMask` = the display ids whose monitor changed, nothing unplugged.
    #[must_use]
    pub fn hotplug(client: u32, event: u32, plug_display_mask: u32) -> SubdeviceNotify {
        let mut event_data = [0u8; 8];
        event_data[..4].copy_from_slice(&plug_display_mask.to_le_bytes());
        SubdeviceNotify {
            client,
            event,
            notify_index: NOTIFIERS_HOTPLUG,
            event_data,
        }
    }

    /// The body: the 32-byte struct with `eventDataSize` = 8 and `bNotifyList` = 1, the data at
    /// [`EVENT_DATA_AT`], 40 bytes in all.
    ///
    /// # Errors
    /// A `notify_index` the guest would use as an out-of-bounds array index (≥
    /// [`NOTIFIERS_MAXCOUNT`] — which includes any index carrying `NV01_EVENT_CLIENT_RM`).
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        if self.notify_index >= NOTIFIERS_MAXCOUNT {
            return Err(format!(
                "a list POST_EVENT with notifyIndex {:#x} would index notifyActions[{NOTIFIERS_MAXCOUNT}] \
                 out of bounds in the guest kernel",
                self.notify_index
            ));
        }
        let mut buf = vec![0u8; RpcPostEventV1700::SIZE + self.event_data.len()];
        let m = RpcPostEventV1700 {
            h_client: self.client,
            h_event: self.event,
            notify_index: self.notify_index,
            data: 0,
            info16: 0,
            status: 0,
            event_data_size: self.event_data.len() as u32,
            b_notify_list: 1,
        };
        m.encode_into(&mut buf[..RpcPostEventV1700::SIZE])
            .map_err(|e| format!("rpc_post_event_v17_00: {e:?}"))?;
        buf[EVENT_DATA_AT..EVENT_DATA_AT + self.event_data.len()].copy_from_slice(&self.event_data);
        Ok(buf)
    }
}

kf_util::assert_send_sync!(SubdeviceNotify);

#[cfg(test)]
mod tests {
    use super::*;

    /// ★ The body the C puts on the wire, field for field
    /// (`C: src/qemu/nvkvm_gpu_emul.c:1806-1821`): `hClient` @ +0, `hEvent` @ +4,
    /// `notifyIndex` @ +8, `data` @ +12, and everything from +16 to +31 zero.
    #[test]
    fn the_encoded_body_is_the_c_artifacts_body() {
        let b = PostEvent {
            client: 0xc1d0_000c,
            event: 0x5c00_0079,
            notify_index: 35,
        }
        .encode();
        assert_eq!(
            b.len(),
            32,
            "sizeof(rpc_post_event_v17_00), empty eventData"
        );
        assert_eq!(&b[0..4], &0xc1d0_000cu32.to_le_bytes());
        assert_eq!(&b[4..8], &0x5c00_0079u32.to_le_bytes());
        assert_eq!(&b[8..12], &35u32.to_le_bytes());
        assert_eq!(&b[12..16], &0u32.to_le_bytes(), "data");
        assert!(
            b[16..32].iter().all(|x| *x == 0),
            "info16, status, eventDataSize and bNotifyList are all zero — an EMPTY, \
             NON-LIST event; see this module's docs for why a fabricated payload is worse \
             than none"
        );
    }

    /// ★ 3c: the hotplug LIST post — bare index 1, `bNotifyList` @ +28 = 1, `eventDataSize` @ +24 =
    /// 8, and the `Nv2080HotplugNotification` at +29 (the flexible array's offset, NOT +32).
    #[test]
    fn the_hotplug_list_post_carries_its_data_at_the_flexible_arrays_offset() {
        let b = SubdeviceNotify::hotplug(0xc1d0_0001, 0xcaf0_0007, 0x100)
            .encode()
            .expect("index 1");
        assert_eq!(b.len(), 40);
        assert_eq!(&b[0..4], &0xc1d0_0001u32.to_le_bytes());
        assert_eq!(&b[4..8], &0xcaf0_0007u32.to_le_bytes());
        assert_eq!(
            &b[8..12],
            &1u32.to_le_bytes(),
            "the bare NV2080_NOTIFIERS_HOTPLUG"
        );
        assert_eq!(&b[24..28], &8u32.to_le_bytes(), "eventDataSize");
        assert_eq!(b[28], 1, "bNotifyList");
        assert_eq!(&b[29..33], &0x100u32.to_le_bytes(), "plugDisplayMask");
        assert_eq!(&b[33..37], &0u32.to_le_bytes(), "unplugDisplayMask");
        let generated = RpcPostEventV1700::LAYOUT
            .fields
            .iter()
            .find(|f| f.c_name == "bNotifyList")
            .expect("bNotifyList");
        assert_eq!(
            generated.offset + generated.width,
            EVENT_DATA_AT,
            "eventData[] follows bNotifyList directly"
        );
    }

    /// ⊘ A list post never carries an index the guest would use out of bounds — in particular not
    /// the registered `HOTPLUG | NV01_EVENT_CLIENT_RM` (0x04000001).
    #[test]
    fn a_list_post_refuses_an_index_at_or_above_maxcount() {
        for idx in [NOTIFIERS_MAXCOUNT, 0x0400_0001, u32::MAX] {
            let p = SubdeviceNotify {
                notify_index: idx,
                ..SubdeviceNotify::hotplug(1, 2, 0x100)
            };
            assert!(p.encode().is_err(), "{idx:#x}");
        }
        let ok = SubdeviceNotify {
            notify_index: NOTIFIERS_MAXCOUNT - 1,
            ..SubdeviceNotify::hotplug(1, 2, 0x100)
        };
        assert!(ok.encode().is_ok());
    }
}
