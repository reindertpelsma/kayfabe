//! ★★★ **VA spaces and channels on the host — the verbs every v3 channel kind is built from.**
//!
//! Passthrough, Translated and VMM-executed emulated channels all reduce to the same host
//! sequence (ogkm-580 `kernel_channel.c`, `kernel_channel_group.c`): a **TSG** bound to a VA
//! space and an engine; a **channel** in it naming a GPFIFO VA + entry count and a USERD
//! (memory object + offset); `BIND`; the **work-submit token**; the **engine object**; then
//! `GPFIFO_SCHEDULE` on the TSG. What differs between the kinds is only WHERE the ring and USERD
//! live — which is the caller's argument, never decided here.
//!
//! Bodies follow the old tree's `birth_channel` / `schedule` / `invalidate_tlb`, which ran on
//! hardware (rm-ladder R14–R17, the thin guest's passthrough channels).

use crate::{ABI_ENCODE_FAILED, HostRm, RmError};
use kf_abi::bringup::{
    NV01_MEMORY_VIRTUAL, NVOS46_FLAGS_DEFER_TLB_INVALIDATION_TRUE, NVOS46_FLAGS_DMA_OFFSET_GROWS_DOWN,
    NVOS47_FLAGS_DEFER_TLB_INVALIDATION_TRUE, NvMemoryVirtualAllocationParams,
    NvVaspaceAllocationParameters,
};
use kf_abi::generated::classes::NvChannelGroupAllocationParameters;
use kf_abi::invariant_classes::{CHANNEL_GROUP, VA_SPACE};
use kf_abi::submit::{
    BIND_PARAMS_SIZE, CeAllocParams, ChannelAllocParams, GpfifoScheduleParams,
    NVA06C_CTRL_CMD_BIND, NVA06C_CTRL_CMD_GPFIFO_SCHEDULE,
    NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN, WORK_SUBMIT_TOKEN_PARAMS_SIZE,
};

/// `NV2080_CTRL_CMD_DMA_INVALIDATE_TLB` (`ogkm-580: ctrl2080dma.h`).
const NV2080_CTRL_CMD_DMA_INVALIDATE_TLB: u32 = 0x2080_2502;
/// `hVASpace` offset inside `NV2080_CTRL_DMA_INVALIDATE_TLB_PARAMS`.
const INVALIDATE_H_VASPACE_OFF: usize = 12;
/// RM requires a USERD offset aligned to 512 bytes (`[measured]`, the old tree's rm.rs:1346).
pub const USERD_ALIGNMENT: u64 = 512;
/// A USERD offset that is not [`USERD_ALIGNMENT`]-aligned — refused BEFORE any host call.
pub const USERD_OFFSET_MISALIGNED: u32 = 0x4B70;

/// One host VA space: the `FERMI_VASPACE_A` object and the `NV01_MEMORY_VIRTUAL` range over it
/// that every map names as `hDma`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaSpace {
    /// The VA space object.
    pub space: u32,
    /// The virtual range (`hDma`).
    pub range: u32,
}

/// One born host channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Channel {
    /// The channel group.
    pub tsg: u32,
    /// The channel.
    pub chan: u32,
    /// The work-submit token its doorbell carries.
    pub token: u32,
}

/// Where a channel's ring and USERD live — always the caller's decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RingSpec {
    /// GPU VA of the GPFIFO, in the channel's VA space.
    pub gp_fifo_va: u64,
    /// GPFIFO entries (a power of two).
    pub gp_fifo_entries: u32,
    /// The memory object holding USERD.
    pub userd_memory: u32,
    /// USERD's offset inside it ([`USERD_ALIGNMENT`]-aligned).
    pub userd_offset: u64,
    /// Error notifier object, or 0.
    pub err_notifier: u32,
}

/// What a mapped object is to the caller — the fact the page-size rule turns on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapBacking {
    /// An object allocated for this one mapping: its base IS the mapping base, RM aligned it.
    Dedicated,
    /// A slice of a shared object whose base we never learn (the store, guest RAM): pinned to
    /// 4 KiB pages so `FIXED` is honoured at every offset, including 0.
    SharedSlice,
}

impl HostRm {
    /// A fresh host VA space and its virtual range.
    ///
    /// # Errors
    /// The host's refusal; a half-built space is freed.
    pub fn alloc_vaspace(&self) -> Result<VaSpace, RmError> {
        let mut params = [0u8; NvVaspaceAllocationParameters::SIZE];
        NvVaspaceAllocationParameters::default()
            .encode_into(&mut params)
            .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        let want = self.mint();
        let space = self.raw_alloc(self.device, want, VA_SPACE, &mut params)?;
        self.remember(space, self.device);
        let mut range = [0u8; NvMemoryVirtualAllocationParams::SIZE];
        NvMemoryVirtualAllocationParams {
            offset: 0,
            limit: 0,
            h_va_space: space,
        }
        .encode_into(&mut range)
        .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        let want = self.mint();
        match self.raw_alloc(self.device, want, NV01_MEMORY_VIRTUAL, &mut range) {
            Ok(h) => {
                self.remember(h, self.device);
                Ok(VaSpace { space, range: h })
            }
            Err(e) => {
                let _ = self.free(space);
                Err(e)
            }
        }
    }

    /// Map `len` bytes of `memory` at `offset` into `space`, at `at` if given. `defer` sets
    /// `DEFER_TLB_INVALIDATION` — ★ v3 §4.2: every map of a batch defers except that the batch
    /// ends with ONE [`HostRm::invalidate_tlb`].
    ///
    /// ★ `backing` is STATED by the caller, never inferred from `offset` (review w826 #5): the
    /// store's first page is a slice at offset 0, and a slice mapped under the dedicated-object
    /// page-size rule has `FIXED` ignored outright (`[measured w755e]`, 4 633 relocations).
    ///
    /// # Errors
    /// The host's refusal, or [`RmError::PlacementRefused`] for a FIXED map RM placed elsewhere.
    pub fn map(
        &self,
        space: VaSpace,
        memory: u32,
        backing: MapBacking,
        offset: u64,
        len: u64,
        at: Option<u64>,
        defer: bool,
    ) -> Result<u64, RmError> {
        let extra = if defer { NVOS46_FLAGS_DEFER_TLB_INVALIDATION_TRUE } else { 0 };
        self.raw_map_dma_slice(space.range, memory, offset, len, at, extra, backing == MapBacking::SharedSlice)
    }

    /// ★ Map ALL of `memory` (`len` bytes) into `space` at an address RM chooses, and return it —
    /// a **window**: the identity window over the store (guest FB-physical `p` ⇒ `base + p`) or
    /// the guest-RAM window. ⊘ The base is READ BACK, never assumed (`THE_TRANSLATED_PLANE.md`
    /// §16.1, §17.1): RM chose `0x120000000` on GA106/580, and a later driver may not.
    /// `high` maps it GROWS_DOWN, away from a guest kernel's bottom-up allocations (§24.2).
    ///
    /// # Errors
    /// The host's refusal.
    pub fn map_window(&self, space: VaSpace, memory: u32, len: u64, high: bool) -> Result<u64, RmError> {
        let extra = if high { NVOS46_FLAGS_DMA_OFFSET_GROWS_DOWN } else { 0 };
        self.raw_map_dma_slice(space.range, memory, 0, len, None, extra, false)
    }

    /// Unmap the mapping at `va` in `space`; `defer` as for [`HostRm::map`].
    ///
    /// # Errors
    /// The host's status.
    pub fn unmap(&self, space: VaSpace, va: u64, defer: bool) -> Result<(), RmError> {
        let flags = if defer { NVOS47_FLAGS_DEFER_TLB_INVALIDATION_TRUE } else { 0 };
        self.raw_unmap_dma_flags(space.range, va, flags)
    }

    /// ONE TLB invalidate for `space` — the end of a deferred batch.
    ///
    /// # Errors
    /// The host's status.
    pub fn invalidate_tlb(&self, space: VaSpace) -> Result<(), RmError> {
        let mut buf = [0u8; 16];
        buf[INVALIDATE_H_VASPACE_OFF..INVALIDATE_H_VASPACE_OFF + 4]
            .copy_from_slice(&space.space.to_le_bytes());
        self.raw_control(self.subdevice, NV2080_CTRL_CMD_DMA_INVALIDATE_TLB, &mut buf)
    }

    /// ★ Birth a host channel: TSG → channel over `ring` → `BIND` → work-submit token. Nothing
    /// is scheduled yet ([`HostRm::schedule`]). A failure frees what was built.
    ///
    /// # Errors
    /// [`USERD_OFFSET_MISALIGNED`] before any host call; else the host's refusal.
    pub fn birth_channel(
        &self,
        space: VaSpace,
        engine_type: u32,
        ring: RingSpec,
    ) -> Result<Channel, RmError> {
        if !ring.userd_offset.is_multiple_of(USERD_ALIGNMENT) {
            return Err(RmError::Other(USERD_OFFSET_MISALIGNED));
        }
        let mut tsg_params = [0u8; NvChannelGroupAllocationParameters::SIZE];
        NvChannelGroupAllocationParameters {
            h_object_error: 0,
            h_object_ecc_error: 0,
            h_va_space: space.space,
            engine_type,
            b_is_calling_context_vgpu_plugin: 0,
        }
        .encode_into(&mut tsg_params)
        .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        let want = self.mint();
        let tsg = self.raw_alloc(self.device, want, CHANNEL_GROUP, &mut tsg_params)?;
        self.remember(tsg, self.device);

        let mut chan_params = [0u8; ChannelAllocParams::SIZE];
        let encoded = ChannelAllocParams {
            h_object_error: ring.err_notifier,
            gp_fifo_offset: ring.gp_fifo_va,
            gp_fifo_entries: ring.gp_fifo_entries,
            flags: 0,
            h_context_share: 0,
            h_va_space: 0,
            h_userd_memory_0: ring.userd_memory,
            userd_offset_0: ring.userd_offset,
            engine_type,
        }
        .encode_into(&mut chan_params);
        if encoded.is_err() {
            let _ = self.free(tsg);
            return Err(RmError::Other(ABI_ENCODE_FAILED));
        }
        let want = self.mint();
        let chan = match self.raw_alloc(tsg, want, self.classes.gpfifo_channel().channel_id().0, &mut chan_params) {
            Ok(h) => h,
            Err(e) => {
                let _ = self.free(tsg);
                return Err(e);
            }
        };
        self.remember(chan, tsg);
        let unwind = |me: &Self| {
            let _ = me.free(chan);
            let _ = me.free(tsg);
        };
        let mut bind = [0u8; BIND_PARAMS_SIZE];
        bind.copy_from_slice(&engine_type.to_le_bytes());
        if let Err(e) = self.raw_control(tsg, NVA06C_CTRL_CMD_BIND, &mut bind) {
            unwind(self);
            return Err(e);
        }
        let mut token = [0u8; WORK_SUBMIT_TOKEN_PARAMS_SIZE];
        if let Err(e) =
            self.raw_control(chan, NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN, &mut token)
        {
            unwind(self);
            return Err(e);
        }
        Ok(Channel {
            tsg,
            chan,
            token: u32::from_le_bytes(token),
        })
    }

    /// The copy-engine object on `chan` (this host's CE class for `engine_type`).
    ///
    /// # Errors
    /// The host's refusal.
    pub fn alloc_ce_object(&self, chan: Channel, engine_type: u32) -> Result<u32, RmError> {
        let mut params = [0u8; CeAllocParams::SIZE];
        CeAllocParams {
            version: CeAllocParams::VERSION_1,
            engine_type,
        }
        .encode_into(&mut params)
        .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        let want = self.mint();
        let h = self.raw_alloc(chan.chan, want, self.classes.ce_object().ce_object_id().0, &mut params)?;
        self.remember(h, chan.chan);
        Ok(h)
    }

    /// `GPFIFO_SCHEDULE` (`bEnable = 1`) on the channel's group — the channel starts fetching.
    ///
    /// # Errors
    /// The host's status.
    pub fn schedule(&self, chan: Channel) -> Result<(), RmError> {
        let mut params = [0u8; GpfifoScheduleParams::SIZE];
        GpfifoScheduleParams {
            b_enable: 1,
            b_skip_submit: 0,
            b_skip_enable: 0,
        }
        .encode_into(&mut params)
        .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        self.raw_control(chan.tsg, NVA06C_CTRL_CMD_GPFIFO_SCHEDULE, &mut params)
    }

    /// Free a channel and its group.
    ///
    /// # Errors
    /// The first host refusal.
    pub fn free_channel(&self, chan: Channel) -> Result<(), RmError> {
        let a = self.free(chan.chan);
        let b = self.free(chan.tsg);
        a.and(b)
    }
}
