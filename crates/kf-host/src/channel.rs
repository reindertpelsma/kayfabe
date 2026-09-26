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
    BIND_PARAMS_SIZE, CeAllocParams, ChannelAllocParams, GpfifoScheduleParams, NvMemoryAllocationParams,
    NVA06C_CTRL_CMD_BIND, NVA06C_CTRL_CMD_GPFIFO_SCHEDULE,
    NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN, WORK_SUBMIT_TOKEN_PARAMS_SIZE,
};

/// `NV01_CONTEXT_DMA` (`ogkm-580: class/cl0002.h:40`).
/// `NVOS04_FLAGS_CHANNEL_DENY_PHYSICAL_MODE_CE_TRUE` at `7:7` (`ogkm-580 alloc_channel.h:168-170`).
pub const NVOS04_FLAGS_CHANNEL_DENY_PHYSICAL_MODE_CE_TRUE: u32 = 1 << 7;

const NV01_CONTEXT_DMA: u32 = 0x0000_0002;
/// `NV2080_CTRL_CMD_DMA_INVALIDATE_TLB` (`ogkm-580: ctrl2080dma.h`).
const NV2080_CTRL_CMD_DMA_INVALIDATE_TLB: u32 = 0x2080_2502;
/// `hVASpace` offset inside `NV2080_CTRL_DMA_INVALIDATE_TLB_PARAMS`.
const INVALIDATE_H_VASPACE_OFF: usize = 12;
/// RM requires a USERD offset aligned to 512 bytes (`[measured]`, the old tree's rm.rs:1346).
pub const USERD_ALIGNMENT: u64 = 512;
/// A USERD offset that is not [`USERD_ALIGNMENT`]-aligned — refused BEFORE any host call.
pub const USERD_OFFSET_MISALIGNED: u32 = 0x4B70;

/// One host VA space: the `FERMI_VASPACE_A` object and the `NV01_MEMORY_VIRTUAL` range over it
/// that every map names as `hDma` — except inside a [`GuestVaRange`], whose own reserving object
/// is the `hDma` there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaSpace {
    /// The VA space object.
    pub space: u32,
    /// The virtual range (`hDma`).
    pub range: u32,
    /// ★ v3-gfx: the guest-allocatable VA ranges, RESERVED in this space (handle 0 = not reserved).
    pub guest: [GuestVaRange; 2],
}

/// ★ v3-gfx — **a VA range reserved in the host space for the GUEST'S mappings.**
///
/// `[measured vgfx 2026-09-26, gfx7]` a GL guest faulted every 3D channel (host Xid 31,
/// `GPCCLIENT_PROP_0 … FAULT_PRIV_VIOLATION`): host RM's own allocator places the twin's GR context
/// buffers (privileged, `bIsKernelAlloc`) lowest-fit in the twin's space — the very VAs the guest's
/// RM, running the same allocator, hands its next surfaces. The guest's FIXED map there then found
/// the VA held by host RM (`VA_ALREADY_MAPPED` ⇒ `HeldByHost`) and its ROP read a host context
/// buffer. RM offers userspace no way to steer its own placements (`VA_INTERNAL_LIMIT` pins them to
/// the split window the client RM reserves against itself; `IS_MIRRORED` is VER1-only). ⇒ kf
/// RESERVES the guest's allocatable ranges up front with a lazy `NV50_MEMORY_VIRTUAL` (no page
/// tables pinned, `gpu_vaspace.c:1640`), so host RM's own placements can only land above them, and
/// maps the guest's rows THROUGH the reservation (a FIXED map into a VA-reserving object is absolute
/// and in-bounds, `dma.c:129-155`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GuestVaRange {
    /// The reserving `NV50_MEMORY_VIRTUAL` handle (0 = none).
    pub handle: u32,
    /// Inclusive start.
    pub lo: u64,
    /// Exclusive end.
    pub hi: u64,
}

/// ★ v3-gfx: the guest-allocatable ranges reserved in every host twin space, leaving ONE hole for
/// host RM's own placements: `[HOST_HOLE_LO, 1 TiB)`.
///
/// - Everything from the split window's end (`4.5 GiB`, `g_gpu_vaspace_nvoc.h:99-100`) — where the
///   guest RM's bottom-up allocator places context buffers and surfaces — up to the hole.
/// - Everything from `1 TiB` to the CPU-VA ceiling `2^47` (UVM places CUDA allocations at CPU VAs).
/// - ⊘ The hole must stay BELOW `1 TiB`: `[measured vgfx 2026-09-26, gfx8]` with the whole of
///   `[4.5 GiB, 2^47)` reserved, host RM placed the twin's GR context buffers above `2^47` and every
///   3D/compute context faulted in context switch (host Xid 44) — GR's global context-buffer
///   pointers are `VA >> 8` in 32-bit fields. kf's own ring region `[1 TiB − 4 GiB, 1 TiB)`
///   (`kf-qemu` `RING_REGION_BASE`) is inside the hole and stays mapped through the range object.
/// - ⊘ `[1 MiB, 4 GiB)` is NOT reserved: `[measured gfx8]` RM refuses a reservation there
///   (`NoMemory`) — it already withholds it — so host RM cannot place there either.
pub const GUEST_VA_RANGES: [(u64, u64); 2] = [((1 << 32) + (1 << 29), HOST_HOLE_LO), (1 << 40, 1 << 47)];
/// The start of host RM's hole — 64 GiB below `1 TiB`. A guest reaches it only after its RM heap has
/// handed out ~1 TiB of VA; a guest row there is mapped through the range object as before (and a
/// collision is still named `HeldByHost`).
pub const HOST_HOLE_LO: u64 = (1 << 40) - (64 << 30);

/// `NV50_MEMORY_VIRTUAL` (`ogkm-580: resource_list.h:516-523`, parent `Device`).
const NV50_MEMORY_VIRTUAL: u32 = 0x50a0;
/// `NVOS32_ALLOC_FLAGS_FIXED_ADDRESS_ALLOCATE | _LAZY | _VIRTUAL` (`nvos.h:1448-1464`).
const NVOS32_RESERVE_FLAGS: u32 = 0x0000_0010 | 0x0000_0400 | 0x0008_0000;

impl VaSpace {
    /// The `hDma` a mapping of `[va, va+len)` names: the reservation that contains it, else the
    /// space's range. ⊘ A mapping straddling a reservation edge is refused by name (the guest's
    /// RM never allocates across the split window, and `2^47` is the CPU-VA ceiling).
    ///
    /// # Errors
    /// [`VA_STRADDLES_RESERVATION`].
    pub fn dma_for(&self, va: u64, len: u64) -> Result<u32, RmError> {
        let end = va.saturating_add(len.max(1));
        for g in self.guest.iter().filter(|g| g.handle != 0) {
            if va >= g.lo && end <= g.hi {
                return Ok(g.handle);
            }
            if va < g.hi && g.lo < end {
                return Err(RmError::Other(VA_STRADDLES_RESERVATION));
            }
        }
        Ok(self.range)
    }
}

/// A FIXED map that straddles the edge of a [`GuestVaRange`].
pub const VA_STRADDLES_RESERVATION: u32 = 0x4B71;

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
                let mut vas = VaSpace { space, range: h, guest: [GuestVaRange::default(); 2] };
                // ★ v3-gfx: reserve the guest's ranges BEFORE anything is placed in the space. A
                // refusal leaves that range unreserved (the pre-v3-gfx behaviour), and says so.
                if std::env::var_os("KF3_NO_GUEST_VA_RESERVE").is_none() {
                    for (slot, &(lo, hi)) in vas.guest.iter_mut().zip(GUEST_VA_RANGES.iter()) {
                        match self.reserve_va(space, lo, hi - lo) {
                            Ok(handle) => *slot = GuestVaRange { handle, lo, hi },
                            Err(e) => eprintln!("kf-host: space {space:#x}: guest VA range [{lo:#x}, {hi:#x}) NOT reserved: {e:?} — host RM may place its own objects there"),
                        }
                    }
                }
                Ok(vas)
            }
            Err(e) => {
                let _ = self.free(space);
                Err(e)
            }
        }
    }

    /// ★ v3-gfx: a lazy, FIXED `NV50_MEMORY_VIRTUAL` reservation of `[at, at+len)` in `space`.
    ///
    /// # Errors
    /// The host's refusal.
    pub fn reserve_va(&self, space: u32, at: u64, len: u64) -> Result<u32, RmError> {
        let mut p = [0u8; NvMemoryAllocationParams::SIZE];
        NvMemoryAllocationParams { owner: self.client.raw(), kind: 0, attr: 0, size: len, alignment: 0 }
            .encode_into(&mut p)
            .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        p[8..12].copy_from_slice(&NVOS32_RESERVE_FLAGS.to_le_bytes());
        p[80..88].copy_from_slice(&at.to_le_bytes()); // offset
        p[108..112].copy_from_slice(&space.to_le_bytes()); // hVASpace
        let want = self.mint();
        let h = self.raw_alloc(self.device, want, NV50_MEMORY_VIRTUAL, &mut p)?;
        self.remember(h, self.device);
        Ok(h)
    }

    /// ★ v3-gfx: free a space and everything kf allocated over it (reservations first — they
    /// reference the space).
    pub fn free_vaspace(&self, space: VaSpace) {
        for g in space.guest.iter().filter(|g| g.handle != 0) {
            let _ = self.free(g.handle);
        }
        let _ = self.free(space.range);
        let _ = self.free(space.space);
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
        let dma = match at {
            Some(a) => space.dma_for(a, len)?,
            None => space.range,
        };
        self.raw_map_dma_slice(dma, memory, offset, len, at, extra, backing == MapBacking::SharedSlice)
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
        self.raw_unmap_dma_flags(space.dma_for(va, 1)?, va, flags)
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
            // ★ P6b: DENY physical-mode CE on EVERY channel we birth (Translated rings and
            // Passthrough twins alike) — `NVOS04_FLAGS_CHANNEL_DENY_PHYSICAL_MODE_CE` 7:7
            // (`ogkm-580 alloc_channel.h:158-170`: "regardless of whether or not the client handle
            // is admin" — the VMM's is). No operand we author is physical (the rewriter turns
            // them into window VAs), so this only ever stops one that ESCAPED the rewriter from
            // reaching host physical memory: `[measured p6b8]` UVM's CE launches on subchannel 4
            // escaped a subchannel-keyed rewriter, verbatim and physical, with no Xid.
            flags: NVOS04_FLAGS_CHANNEL_DENY_PHYSICAL_MODE_CE_TRUE,
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

    /// The family's COMPUTE object under a GR channel — RM builds the channel's GR context
    /// (golden image, context buffers) as part of this alloc. Returns `(handle, class)`.
    /// `NV_GR_ALLOCATION_PARAMETERS` is `{version = 2, flags, size = 16, caps}`
    /// (`ogkm-580: nvos.h:2716-2721`); the construct path reads none of it, CUDA passes it anyway.
    ///
    /// # Errors
    /// The host's refusal, or [`RmError::Other`] when the family declares no compute class.
    pub fn alloc_compute_object(&self, chan: Channel) -> Result<(u32, u32), RmError> {
        let class = self
            .classes
            .compute_object()
            .ok_or(RmError::Other(crate::NOT_ON_THIS_RUNG))?
            .compute_object_id()
            .0;
        let mut params = [0u8; 16];
        params[0..4].copy_from_slice(&2u32.to_le_bytes());
        params[8..12].copy_from_slice(&16u32.to_le_bytes());
        let want = self.mint();
        let h = self.raw_alloc(chan.chan, want, class, &mut params)?;
        self.remember(h, chan.chan);
        Ok((h, class))
    }

    /// ★ P5b: an engine object of `class` on `chan`, with params WE author: a copy class gets
    /// `NVB0B5_ALLOCATION_PARAMETERS {version 1, engineType = the twin's engine}`, a compute/3D
    /// class `NV_GR_ALLOCATION_PARAMETERS {version 2, size 16}` (as [`HostRm::alloc_compute_object`]).
    /// ⊘ The class is the one the guest's pushbuffer will `SET_OBJECT` (so it must be the guest's),
    /// and the caller has already checked it against the host family's generated set; nothing else
    /// of the guest's alloc reaches the host.
    ///
    /// # Errors
    /// The host's refusal.
    pub fn alloc_engine_object(&self, chan: Channel, class: u32, copy_engine: Option<u32>) -> Result<u32, RmError> {
        let mut ce = [0u8; CeAllocParams::SIZE];
        let mut gr = [0u8; 16];
        let params: &mut [u8] = match copy_engine {
            Some(engine_type) => {
                CeAllocParams { version: CeAllocParams::VERSION_1, engine_type }
                    .encode_into(&mut ce)
                    .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
                &mut ce
            }
            None => {
                gr[0..4].copy_from_slice(&2u32.to_le_bytes());
                gr[8..12].copy_from_slice(&16u32.to_le_bytes());
                &mut gr
            }
        };
        let want = self.mint();
        let h = self.raw_alloc(chan.chan, want, class, params)?;
        self.remember(h, chan.chan);
        Ok(h)
    }

    /// ★ w827: a `GT200_DEBUGGER` session on OUR device, bound to `obj3d` — a GR object this
    /// session allocated (a twin's engine object). Params WE author:
    /// `NV83DE_ALLOC_PARAMETERS {hDebuggerClient_Obsolete = 0, hAppClient = our client,
    /// hClass3dObject = obj3d}` (`ogkm-580: class/cl83de.h:51-55`); nothing of the guest's alloc
    /// reaches the host but the fact that it asked for one. Unprivileged
    /// (`RS_FLAGS_ALLOC_NON_PRIVILEGED`, `resource_list.h:192`).
    ///
    /// # Errors
    /// The host's refusal.
    pub fn alloc_debugger(&self, obj3d: u32) -> Result<u32, RmError> {
        let mut params = [0u8; 12];
        params[4..8].copy_from_slice(&self.client.raw().to_le_bytes());
        params[8..12].copy_from_slice(&obj3d.to_le_bytes());
        let want = self.mint();
        let h = self.raw_alloc(self.device, want, 0x83de, &mut params)?;
        self.remember(h, self.device);
        Ok(h)
    }

    /// ★ w827: `NV83DE_CTRL_CMD_DEBUG_SET_EXCEPTION_MASK` on one of our debugger sessions — a
    /// 4-byte event filter inside RM, no hardware write (`ctrl83dedebug.h:158-231`).
    ///
    /// # Errors
    /// The host's status.
    pub fn debugger_set_exception_mask(&self, debugger: u32, mask: u32) -> Result<(), RmError> {
        let mut params = mask.to_le_bytes();
        self.raw_control(debugger, 0x83de_0309, &mut params)
    }

    /// ★ w827: `NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE` for `chan`'s GROUP, on our
    /// subdevice, with the guest's requested `flags`/modes and no route (`ctrl2080gr.h:818-842`).
    /// Unprivileged (`NON_PRIVILEGED`, `g_subdevice_nvoc.c`); host RM validates the modes.
    ///
    /// # Errors
    /// The host's status.
    pub fn set_ctxsw_preemption_mode(&self, chan: Channel, flags: u32, gfxp: u32, cilp: u32) -> Result<(), RmError> {
        let mut p = [0u8; 32];
        p[0..4].copy_from_slice(&flags.to_le_bytes());
        p[4..8].copy_from_slice(&chan.tsg.to_le_bytes());
        p[8..12].copy_from_slice(&gfxp.to_le_bytes());
        p[12..16].copy_from_slice(&cilp.to_le_bytes());
        self.raw_control(self.subdevice, 0x2080_1210, &mut p)
    }

    /// ★ v3-gfx: `NV2080_CTRL_CMD_GR_CTXSW_ZCULL_BIND` for `chan` on our subdevice — params WE
    /// author: our client, the twin's channel, the guest's zcull buffer VA (the twin's VA space is
    /// the guest channel's, VA-identical) and a mode the caller validated (`0..=2`). Host RM binds
    /// it into the twin's GR context; it programs nothing outside that context
    /// (`ctrl2080gr.h:589-608`).
    ///
    /// # Errors
    /// The host's status.
    pub fn zcull_bind(&self, chan: Channel, va: u64, mode: u32) -> Result<(), RmError> {
        let mut p = [0u8; 24];
        p[0..4].copy_from_slice(&self.client.raw().to_le_bytes());
        p[4..8].copy_from_slice(&chan.chan.to_le_bytes());
        p[8..16].copy_from_slice(&va.to_le_bytes());
        p[16..20].copy_from_slice(&mode.to_le_bytes());
        self.raw_control(self.subdevice, 0x2080_1208, &mut p)
    }

    /// ★ w827: `NVA06C_CTRL_CMD_SET_TIMESLICE` on `chan`'s group (`ctrla06c.h:146-152`) — host RM
    /// rounds to what the hardware supports and refuses what it does not.
    ///
    /// # Errors
    /// The host's status.
    pub fn set_timeslice(&self, chan: Channel, us: u64) -> Result<(), RmError> {
        let mut p = us.to_le_bytes();
        self.raw_control(chan.tsg, 0xa06c_0103, &mut p)
    }

    /// ★ w827: `NV0080_CTRL_CMD_PERF_CUDA_LIMIT_SET_CONTROL` (`0x00801909`) on OUR device —
    /// the unprivileged userspace verb whose internal consequence is the GSP's CUDA-limit edge
    /// (`kern_cuda_limit.c:88-128`); host RM refcounts it on our Device.
    ///
    /// # Errors
    /// The host's status.
    pub fn perf_cuda_limit(&self, enable: bool) -> Result<(), RmError> {
        let mut p = [u8::from(enable)];
        self.raw_control(self.device, 0x0080_1909, &mut p)
    }

    /// ★ w827: `NV2080_CTRL_CMD_FB_FLUSH_GPU_CACHE` (`0x2080130e`, `NON_PRIVILEGED`) on OUR
    /// subdevice, `FLUSH_MODE_FULL_CACHE`, with the aperture and write-back/invalidate named —
    /// `kmemsysFlushGpuCache` maps these onto the L2 registers (`kern_mem_sys_ctrl.c`,
    /// `kmemsysCacheOp_GM200`). Params (`ctrl2080fb.h:640-646`): `NvU64 addressArray[500]` @0,
    /// `addressArraySize` @4000, `addressAlign` @4004, `NvU64 memBlockSizeBytes` @4008, `flags`
    /// @4016 — 4024 bytes with the struct's 8-byte tail padding.
    ///
    /// # Errors
    /// The host's status.
    pub fn flush_gpu_cache(&self, aperture: u32, write_back: bool, invalidate: bool) -> Result<(), RmError> {
        const SIZE: usize = 4024;
        let flags = (aperture & 0x3) | (u32::from(write_back) << 2) | (u32::from(invalidate) << 3) | (1 << 4);
        let mut p = vec![0u8; SIZE];
        p[4016..4020].copy_from_slice(&flags.to_le_bytes());
        self.raw_control(self.subdevice, 0x2080_130e, &mut p)
    }

    /// `GPFIFO_SCHEDULE` (`bEnable = 1`) on the channel's group — the channel starts fetching.
    ///
    /// # Errors
    /// The host's status.
    pub fn schedule(&self, chan: Channel) -> Result<(), RmError> {
        self.schedule_enable(chan, true)
    }

    /// `GPFIFO_SCHEDULE` on the channel's group with `bEnable = enable` (P5b: the guest's own
    /// schedule statement, carried to its twin — the flag is the guest's intent, the verb ours).
    ///
    /// # Errors
    /// The host's status.
    pub fn schedule_enable(&self, chan: Channel, enable: bool) -> Result<(), RmError> {
        let mut params = [0u8; GpfifoScheduleParams::SIZE];
        GpfifoScheduleParams {
            b_enable: u8::from(enable),
            b_skip_submit: 0,
            b_skip_enable: 0,
        }
        .encode_into(&mut params)
        .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        self.raw_control(chan.tsg, NVA06C_CTRL_CMD_GPFIFO_SCHEDULE, &mut params)
    }

    /// ★★★ v3-chanctl — `NVA06C_CTRL_CMD_PREEMPT` (`0xa06c0105`) on the channel's own host
    /// group, `bWait = 1` (AUTHORED: the reply we hold is the preempt's completion, never an
    /// "issued" — `ctrla06c.h:177-198`), RM's default timeout (no manual one).
    ///
    /// ⊘ **Unprivileged**: its export flags are `0x10248` (`ogkm-580:
    /// g_kernel_channel_group_api_nvoc.c:273`) — `RMCTRL_FLAGS_NON_PRIVILEGED` (`0x8`,
    /// `control.h:208`) set, neither `PRIVILEGED` (`0x4`) nor `INTERNAL` (`0x80`); issued on OUR
    /// client's own group.
    ///
    /// # Errors
    /// The host's status.
    pub fn preempt(&self, chan: Channel) -> Result<(), RmError> {
        let mut p = kf_abi::submit::Preempt { wait: true, manual_timeout: false, timeout_us: 0 }.encode();
        self.raw_control(chan.tsg, kf_abi::submit::NVA06C_CTRL_CMD_PREEMPT, &mut p)
    }

    /// ★★★ v3-chanctl — `NV2080_CTRL_CMD_FIFO_DISABLE_CHANNELS` (`0x2080110b`) over OUR channels
    /// (every entry names this client). `disable && !only_scheduling` is the documented
    /// "none of the listed channels are running in hardware and will not run until a call with
    /// `bDisable=NV_FALSE`" (`ctrl2080fifo.h:309-316`); `only_scheduling` degrades it to "not
    /// scheduled"; `!disable` undoes it. `pRunlistPreemptEvent` is always NULL.
    ///
    /// ⊘ **Unprivileged**: export flags `0x10108` (`ogkm-580: g_subdevice_nvoc.c:4921`) —
    /// `NON_PRIVILEGED` set; the one privilege check in its CPU-RM body is on
    /// `pRunlistPreemptEvent` (`kernel_fifo_ctrl.c:720-725`), which we never pass.
    ///
    /// # Errors
    /// The host's status; more than 64 channels.
    pub fn disable_channels(&self, chans: &[Channel], disable: bool, only_scheduling: bool, rewind_gp_put: bool) -> Result<(), RmError> {
        let d = kf_abi::submit::DisableChannels {
            disable,
            only_disable_scheduling: only_scheduling,
            rewind_gp_put,
            runlist_preempt_event: 0,
            list: chans.iter().map(|c| (self.client.raw(), c.chan)).collect(),
        };
        let mut p = d.encode().map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        self.raw_control(self.subdevice, kf_abi::submit::NV2080_CTRL_CMD_FIFO_DISABLE_CHANNELS, &mut p)
    }

    /// ★ Is host copy engine `engine_type` a GRAPHICS copy engine (it shares the GR runlist)?
    /// `NV2080_CTRL_CMD_CE_GET_CAPS_V2` (`0x20802a03`, `ogkm-580: ctrl2080ce.h:78-91`):
    /// `capsTbl[0] & NV2080_CTRL_CE_CAPS_CE_GRCE`. ⊘ Asked of the HOST, never assumed from a mask:
    /// a channel on a GRCE's runlist routes subchannels 0-3 to GR, so a CE pushbuffer on
    /// subchannel 0 (RM's own `RM_SUBCHANNEL`) faults `CTXNOTVALID` there (`[measured p5f]`, Xid 32).
    ///
    /// # Errors
    /// The host's refusal (e.g. an engine the host does not have).
    pub fn ce_is_grce(&self, engine_type: u32) -> Result<bool, RmError> {
        let mut p = [0u8; 8];
        p[0..4].copy_from_slice(&engine_type.to_le_bytes());
        self.raw_control(self.subdevice, 0x2080_2a03, &mut p)?;
        Ok(p[4] & 0x01 != 0)
    }

    /// ★ P5c: an `NV01_CONTEXT_DMA` over `[offset, offset+len)` of `memory` (a device-parented
    /// object: the store, or the guest-RAM descriptor) — the error context a host twin names so
    /// that the host's RC path writes the GUEST's own notifier record natively.
    ///
    /// `NV_CONTEXT_DMA_ALLOCATION_PARAMS {hSubDevice, flags, hMemory, offset, limit}`
    /// (`ogkm-580: nvos.h:1595-1602`); `flags` is authored: read/write, snoop, and
    /// `HASH_TABLE_DISABLE` (`context_dma.c:224-229` refuses ENABLE). ⊘ No `TYPE_NOTIFIER`: that
    /// asks the host CPU-RM for a kernel mapping of the range, and the writer here is the host's
    /// GSP (`kernel_gsp.c:541-545`, `kernel_rc_notification.c:85-89`), never its CPU.
    ///
    /// # Errors
    /// The host's refusal (`NV_ERR_INVALID_LIMIT` past the object).
    pub fn alloc_context_dma(&self, memory: u32, offset: u64, len: u64) -> Result<u32, RmError> {
        const NVOS03_FLAGS_HASH_TABLE_DISABLE: u32 = 1 << 29;
        let limit = len.checked_sub(1).ok_or(RmError::Other(ABI_ENCODE_FAILED))?;
        let mut p = [0u8; 32];
        p[4..8].copy_from_slice(&NVOS03_FLAGS_HASH_TABLE_DISABLE.to_le_bytes());
        p[8..12].copy_from_slice(&memory.to_le_bytes());
        p[16..24].copy_from_slice(&offset.to_le_bytes());
        p[24..32].copy_from_slice(&limit.to_le_bytes());
        let want = self.mint();
        let h = self.raw_alloc(self.device, want, NV01_CONTEXT_DMA, &mut p)?;
        self.remember(h, self.device);
        Ok(h)
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
