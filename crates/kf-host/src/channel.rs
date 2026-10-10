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

use crate::{ABI_ENCODE_FAILED, DISP_SW_CLASS_ID_UNREADABLE, HostRm, RmError};
use kf_abi::bringup::{
    NV01_MEMORY_VIRTUAL, NVOS46_FLAGS_ACCESS_READ_ONLY, NVOS46_FLAGS_DEFER_TLB_INVALIDATION_TRUE,
    NVOS46_FLAGS_DMA_OFFSET_GROWS_DOWN, NVOS46_FLAGS_GPU_CACHEABLE_NO,
    NVOS46_FLAGS_PAGE_KIND_OVERRIDE_YES, NVOS46_FLAGS_TLB_LOCK_ENABLE,
    NVOS47_FLAGS_DEFER_TLB_INVALIDATION_TRUE, NvMemoryVirtualAllocationParams,
    NvVaspaceAllocationParameters,
};
use kf_abi::generated::classes::NvChannelGroupAllocationParameters;
use kf_abi::invariant_classes::{CHANNEL_GROUP, VA_SPACE};
use kf_abi::submit::{
    BIND_PARAMS_SIZE, CeAllocParams, ChannelAllocParams, GpfifoScheduleParams,
    NVA06C_CTRL_CMD_BIND, NVA06C_CTRL_CMD_GPFIFO_SCHEDULE,
    NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN, NvMemoryAllocationParams,
    WORK_SUBMIT_TOKEN_PARAMS_SIZE,
};

/// `NV01_CONTEXT_DMA` (`ogkm-580: class/cl0002.h:40`).
/// `NVOS04_FLAGS_CHANNEL_DENY_PHYSICAL_MODE_CE_TRUE` at `7:7` (`ogkm-580 alloc_channel.h:168-170`).
pub const NVOS04_FLAGS_CHANNEL_DENY_PHYSICAL_MODE_CE_TRUE: u32 = 1 << 7;
/// `NVOS04_FLAGS_PRIVILEGED_CHANNEL_TRUE` at `5:5` (`ogkm-580: alloc_channel.h:141-143`) — the bit
/// host RM SETS in the alloc reply when it stamps a channel `ADMIN` or `KERNEL`
/// (`kernel_channel.c:278-287`). kf3 requests it clear and reads it back ([`birth_privilege`]).
pub const NVOS04_FLAGS_PRIVILEGED_CHANNEL_TRUE: u32 = 1 << 5;
/// `flags` @ +20 of `NV_CHANNEL_ALLOC_PARAMS` — inside the +0..+32 prefix that 580 and 610 spell
/// identically (`kf_abi::versions::DriverAbiTable::decode_channel_alloc_facts`).
const CHANNEL_ALLOC_FLAGS_OFF: usize = 20;

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
    /// Slots 0-1: [`GUEST_VA_RANGES`]; slot 2: [`MirrorVaStart::low_range`] (2026-10-08, only in a
    /// space started at [`TWIN_VA_FLOOR`]).
    pub guest: [GuestVaRange; 3],
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
pub const GUEST_VA_RANGES: [(u64, u64); 2] =
    [((1 << 32) + (1 << 29), HOST_HOLE_LO), (1 << 40, 1 << 47)];
/// The start of host RM's hole — 64 GiB below `1 TiB`. A guest reaches it only after its RM heap has
/// handed out ~1 TiB of VA; a guest row there is mapped through the range object as before (and a
/// collision is still named `HeldByHost`).
pub const HOST_HOLE_LO: u64 = (1 << 40) - (64 << 30);

/// `NV50_MEMORY_VIRTUAL` (`ogkm-580: resource_list.h:516-523`, parent `Device`).
const NV50_MEMORY_VIRTUAL: u32 = 0x50a0;
/// `NVOS32_ALLOC_FLAGS_FIXED_ADDRESS_ALLOCATE | _LAZY | _VIRTUAL` (`nvos.h:1448-1464`).
const NVOS32_RESERVE_FLAGS: u32 = 0x0000_0010 | 0x0000_0400 | 0x0008_0000;

impl VaSpace {
    /// ★ **STATUS (2026-10-09): this is the ONE-handle form; a straddling row is no longer refused
    /// by the map/unmap verbs — they split it with [`VaSpace::dma_pieces`].** `[measured, run 225,
    /// real GPU, 2026-10-09]` Windows' guest maps rows that CROSS the 1 MiB edge of the low
    /// reservation (`map 0xb0000+0x80000`, `map 0xff000+0x2000`); refusing them here
    /// ([`VA_STRADDLES_RESERVATION`], `0x4B71` = 19313) left the VA unmapped on the twin, the GPU
    /// used it and faulted (host Xid 31, `FAULT_PTE` at `0xff000` / `0xb0000`), and the Windows
    /// driver died. A FIXED `NV_ESC_RM_MAP_MEMORY_DMA` names ONE `hDma`, so such a row is TWO maps
    /// of the same memory object, one per `hDma` ([`VaSpace::dma_pieces`]).
    ///
    /// (Superseded text, kept for the callers that still need one handle:) The `hDma` a mapping of
    /// `[va, va+len)` names: the reservation that contains it, else the space's range. ⊘ A mapping
    /// straddling a reservation edge is refused by name here (the guest's RM never allocates across
    /// the split window, and `2^47` is the CPU-VA ceiling) — [`HostRm::map_kind`],
    /// [`HostRm::unmap_row`] and [`HostRm::unmap_range`] do NOT call this; they call
    /// [`VaSpace::dma_pieces`].
    ///
    /// # Errors
    /// [`VA_STRADDLES_RESERVATION`]; [`VA_BELOW_TWIN_FLOOR`] and [`VA_ROW_WRAPS`]
    /// ([`guest_row_end`]).
    pub fn dma_for(&self, va: u64, len: u64) -> Result<u32, RmError> {
        let end = guest_row_end(va, len)?;
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

    /// ★ **STATUS (2026-10-09, LIVE): split a guest row at the reservation edges, one piece per
    /// `hDma`.** Returns `(hDma, va, len)` pieces that are contiguous, in order, and cover exactly
    /// `[va, va+len)`; each lies wholly inside ONE live guest reservation (that reservation's
    /// handle) or wholly outside every reservation (the space's [`VaSpace::range`]). A row inside
    /// one object is exactly one piece — `(dma_for(va, len), va, len)`, no behaviour change. A
    /// zero-length row is the one-byte convention of [`guest_row_end`], returned as one piece of
    /// length 0.
    ///
    /// Hostile input: `va` and `len` are untrusted. The arithmetic is checked, the loop makes
    /// progress every turn and is bounded by [`VA_PIECES_MAX`] (`2 * reservations + 1`), so no
    /// guest value sizes an allocation, and a huge or reversed row is refused by name before any
    /// host call.
    ///
    /// # Errors
    /// [`VA_BELOW_TWIN_FLOOR`] and [`VA_ROW_WRAPS`] ([`guest_row_end`]); [`VA_PIECE_UNPLACEABLE`]
    /// for a piece no `hDma` can take (the space has no range object, or the bound is exceeded).
    pub fn dma_pieces(&self, va: u64, len: u64) -> Result<Vec<(u32, u64, u64)>, RmError> {
        let end = guest_row_end(va, len)?;
        let mut out = Vec::with_capacity(VA_PIECES_MAX);
        let mut cur = va;
        while cur < end {
            if out.len() >= VA_PIECES_MAX {
                return Err(RmError::Other(VA_PIECE_UNPLACEABLE));
            }
            let live = || self.guest.iter().filter(|g| g.handle != 0 && g.lo < g.hi);
            let (handle, stop) = match live().find(|g| g.lo <= cur && cur < g.hi) {
                Some(g) => (g.handle, g.hi.min(end)),
                None => {
                    let next = live().filter(|g| g.lo > cur).map(|g| g.lo).min();
                    (self.range, next.unwrap_or(end).min(end))
                }
            };
            if handle == 0 || stop <= cur {
                return Err(RmError::Other(VA_PIECE_UNPLACEABLE));
            }
            out.push((handle, cur, stop - cur));
            cur = stop;
        }
        if len == 0
            && let [(_, _, l)] = out.as_mut_slice()
        {
            *l = 0;
        }
        Ok(out)
    }

    /// ★ v3-int: whether `[va, va+len)` lies wholly inside a LIVE guest reservation — i.e. host
    /// RM's own allocator can never place anything there (only the guest's FIXED maps land in it).
    #[must_use]
    pub fn guest_reserved(&self, va: u64, len: u64) -> bool {
        let end = va.saturating_add(len.max(1));
        self.guest
            .iter()
            .any(|g| g.handle != 0 && va >= g.lo && end <= g.hi)
    }
}

/// A FIXED map that straddles the edge of a [`GuestVaRange`] — refused by [`VaSpace::dma_for`] (the
/// one-handle form) only. ★ STATUS (2026-10-09): [`HostRm::map_kind`] / [`HostRm::unmap_row`] /
/// [`HostRm::unmap_range`] no longer refuse it; they split the row ([`VaSpace::dma_pieces`]).
/// `[measured, run 225]` the refusal (`Other(19313)`) was the cause of Xid 31 at `0xff000` /
/// `0xb0000`.
pub const VA_STRADDLES_RESERVATION: u32 = 0x4B71;
/// A piece of a guest row that no `hDma` can take ([`VaSpace::dma_pieces`]): the space has no range
/// object, or the piece count exceeds [`VA_PIECES_MAX`] (cannot happen with well-formed
/// reservations) — refused before any host call.
pub const VA_PIECE_UNPLACEABLE: u32 = 0x4B79;
/// The most pieces [`VaSpace::dma_pieces`] returns: a row alternates outside-gap / reservation, and
/// [`VaSpace::guest`] holds three reservations, so at most `3 + 4 = 2 * 3 + 1`.
pub const VA_PIECES_MAX: usize = 2 * 3 + 1;
/// A guest row inside the NULL big page `[0, TWIN_VA_FLOOR)` — refused before any host call.
pub const VA_BELOW_TWIN_FLOOR: u32 = 0x4B77;
/// A guest row whose end does not fit in 64 bits (`va + len` wraps) — refused before any host call.
pub const VA_ROW_WRAPS: u32 = 0x4B78;

/// ★ 2026-10-08 (Windows walls, task A) — **where a guest-mirror twin's host VA space starts.**
///
/// The rule. A twin's host space mirrors the guest's page tables, so it must accept every VA those
/// tables can name — and that is NOT bounded by any RM's allocator start:
/// - host RM starts a space at `vaStartMin` = 1 MiB unless `vaBase` is given
///   (`ogkm-595.84: gpu_vaspace.c:1105,1158-1165`, `g_gpu_vaspace_nvoc.h:781-785`); the guest's own RM
///   computes the same start for ITS allocator;
/// - but a `SHARED_MANAGEMENT` space (`NV_VASPACE_ALLOCATION_FLAGS` bit 2, `nvos.h:3162`) lets the OS
///   hook its own PDEs beneath RM's root (`gpu_vaspace.c:1138-1145`), and Windows does: `[measured,
///   run60/run70 at 3a578d50/4b14d74f, 2026-10-08]` every Windows process space is allocated with
///   flags `0x5` (MINIMIZE_PTETABLE_SIZE | SHARED_MANAGEMENT), and `[measured, run69 at d67e9290,
///   2026-10-08]` its page tables map VA `0x10000` (a host space starting at 1 MiB refused that row,
///   `rangeLo <= rangeHi @ gpu_vaspace.c:1363`, and the twin faulted at `0x13000`, Xid 31).
///
/// So the start is the first non-NULL big page: [`TWIN_VA_FLOOR`] (64 KiB = `NV_VASPACE_BIG_PAGE_SIZE_64K`,
/// the big page size Windows' process spaces declare, `[measured, run70]` `bigPageSize=0x10000`; RM
/// documents `vaBase` as aligned to the space's largest page size, `nvos.h:3119-3123`, and the
/// constructor does not check it, `gpu_vaspace.c:1158-1165`; `[measured, runs 70-72 at
/// 4b14d74f..8de8ef26, 2026-10-08]` host RM 595.91.07 accepts `0x10000`). Nothing the guest declares moves it: the
/// guest's `vaBase`/`vaSize` describe ITS RM's allocator window, not the reach of its page tables, and a
/// guest-chosen base could only ever refuse its own rows. The NULL big page `[0, 64 KiB)` stays outside
/// every twin space; a row there is refused by name ([`VA_BELOW_TWIN_FLOOR`]).
///
/// What keeps host RM's OWN buffers out of the guest's low range: `[TWIN_VA_FLOOR, 1 MiB)` is reserved
/// for guest rows exactly like [`GUEST_VA_RANGES`] ([`MirrorVaStart::low_range`]), so host RM's own
/// placements land where they did with the default start. The range is a constant — never widened by
/// guest input — and lies wholly below host RM's default start, so the space's host-side layout is
/// otherwise unchanged.
///
/// Default OFF: applied only when a twin can run in a `SHARED_MANAGEMENT` space at all, i.e. under
/// `KF3_WIN_USER_CHANNELS_PASSTHROUGH=1` (Windows per-process work, `OWNER_RULINGS.md` §V). Linux
/// guests keep host RM's default start. It replaces the diagnostic `KF3_TWIN_VA_BASE` (runs 70-72).
pub const TWIN_VA_FLOOR: u64 = 0x1_0000;
/// Host RM's default VA-space start (`gvaspaceGetReservedVaspaceBase`, 1 MiB on a non-MIG GPU).
pub const HOST_DEFAULT_VA_START: u64 = 0x10_0000;

/// Where a guest-mirror host space starts ([`TWIN_VA_FLOOR`] explains the rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirrorVaStart {
    /// Host RM's default (`vaBase = 0` → 1 MiB): every guest whose page tables stay above it.
    HostDefault,
    /// [`TWIN_VA_FLOOR`], with `[TWIN_VA_FLOOR, HOST_DEFAULT_VA_START)` reserved for guest rows.
    GuestFloor,
}

impl MirrorVaStart {
    /// The rule's switch: [`MirrorVaStart::GuestFloor`] iff Windows per-process work may run on twins
    /// (`KF3_WIN_USER_CHANNELS_PASSTHROUGH=1`, default off).
    #[must_use]
    pub fn from_env() -> Self {
        Self::select(std::env::var("KF3_WIN_USER_CHANNELS_PASSTHROUGH").as_deref() == Ok("1"))
    }

    /// Pure form of [`MirrorVaStart::from_env`].
    #[must_use]
    pub const fn select(windows_user_twins: bool) -> Self {
        if windows_user_twins {
            Self::GuestFloor
        } else {
            Self::HostDefault
        }
    }

    /// The `vaBase` to request (0 = host RM's default).
    #[must_use]
    pub const fn va_base(self) -> u64 {
        match self {
            Self::HostDefault => 0,
            Self::GuestFloor => TWIN_VA_FLOOR,
        }
    }

    /// The low guest range to reserve before host RM places anything, if any.
    #[must_use]
    pub const fn low_range(self) -> Option<(u64, u64)> {
        match self {
            Self::HostDefault => None,
            Self::GuestFloor => Some((TWIN_VA_FLOOR, HOST_DEFAULT_VA_START)),
        }
    }
}

/// The exclusive end of a guest row `[va, va+len)`, refusing by name a row in the NULL big page or one
/// whose end wraps. A zero-length row is treated as one byte (as before).
///
/// # Errors
/// [`VA_BELOW_TWIN_FLOOR`], [`VA_ROW_WRAPS`].
pub fn guest_row_end(va: u64, len: u64) -> Result<u64, RmError> {
    if va < TWIN_VA_FLOOR {
        return Err(RmError::Other(VA_BELOW_TWIN_FLOOR));
    }
    va.checked_add(len.max(1))
        .ok_or(RmError::Other(VA_ROW_WRAPS))
}

/// The `NVOS47` flags of an unmap (`defer`: leave the TLB invalidate to the batch's one).
const fn unmap_flags(defer: bool) -> u32 {
    if defer {
        NVOS47_FLAGS_DEFER_TLB_INVALIDATION_TRUE
    } else {
        0
    }
}

/// The arguments of one FIXED map ([`DmaVerbs::map_one`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MapPiece {
    pub(crate) memory: u32,
    pub(crate) offset: u64,
    pub(crate) len: u64,
    pub(crate) at: u64,
    pub(crate) extra: u32,
    pub(crate) shared: bool,
    pub(crate) kind: u32,
}

/// The two host verbs a split row is built from — [`HostRm`] in production, a recorder in tests.
pub(crate) trait DmaVerbs {
    /// One FIXED `NV_ESC_RM_MAP_MEMORY_DMA` of `[offset, offset+len)` of `memory` at `at` in
    /// `h_dma`, asserting its own placement ([`HostRm::raw_map_dma_slice`]).
    fn map_one(&self, h_dma: u32, p: MapPiece) -> Result<u64, RmError>;
    /// One `NV_ESC_RM_UNMAP_MEMORY_DMA` in `h_dma`: `size == 0` the whole mapping keyed by its exact
    /// start `va`, else every mapping intersecting `[va, va+size)`.
    fn unmap_one(&self, h_dma: u32, va: u64, size: u64, flags: u32) -> Result<(), RmError>;
}

impl DmaVerbs for HostRm {
    fn map_one(&self, h_dma: u32, p: MapPiece) -> Result<u64, RmError> {
        self.raw_map_dma_slice(
            h_dma,
            p.memory,
            p.offset,
            p.len,
            Some(p.at),
            p.extra,
            p.shared,
            p.kind,
        )
    }
    fn unmap_one(&self, h_dma: u32, va: u64, size: u64, flags: u32) -> Result<(), RmError> {
        self.raw_unmap_dma_range(h_dma, va, size, flags)
    }
}

/// ★ **Map a FIXED row as one map per `hDma` it touches, all or nothing** ([`HostRm::map_kind`]).
/// `row.offset` is the offset of `row.at` inside the memory object; piece `i` maps
/// `row.offset + (piece.va - row.at)`. A row inside one object is ONE call with the row's own
/// arguments (no behaviour change). On a refused piece the pieces already mapped are unmapped
/// again (by exact start, with `rollback_flags`, newest first) and the piece's error returned.
pub(crate) fn map_pieces<V: DmaVerbs + ?Sized>(
    v: &V,
    space: VaSpace,
    row: MapPiece,
    rollback_flags: u32,
) -> Result<u64, RmError> {
    let pieces = space.dma_pieces(row.at, row.len)?;
    if let [(h, _, len)] = pieces.as_slice() {
        return v.map_one(*h, MapPiece { len: *len, ..row });
    }
    // Checked once, so no piece offset below can overflow (`piece.va - at < len`).
    row.offset
        .checked_add(row.len)
        .ok_or(RmError::Other(VA_ROW_WRAPS))?;
    for (i, &(h, va, len)) in pieces.iter().enumerate() {
        let piece = MapPiece {
            offset: row.offset + (va - row.at),
            len,
            at: va,
            ..row
        };
        if let Err(e) = v.map_one(h, piece) {
            for &(h0, va0, _) in pieces[..i].iter().rev() {
                if let Err(r) = v.unmap_one(h0, va0, 0, rollback_flags) {
                    eprintln!(
                        "kf-host: split map {:#x}+{:#x}: piece {va:#x} refused ({e:?}); rolling back piece {va0:#x} was refused too ({r:?})",
                        row.at, row.len
                    );
                }
            }
            return Err(e);
        }
    }
    Ok(row.at)
}

/// Unmap `[va, va+len)` piece by piece. `by_range == false`: the whole-mapping unmap keyed by each
/// piece's exact start ([`HostRm::unmap_row`]); `true`: a range unmap over each piece
/// ([`HostRm::unmap_range`]). Every piece is attempted; the first refusal is returned.
pub(crate) fn unmap_pieces<V: DmaVerbs + ?Sized>(
    v: &V,
    space: VaSpace,
    va: u64,
    len: u64,
    by_range: bool,
    flags: u32,
) -> Result<(), RmError> {
    let mut first = Ok(());
    for (h, pva, plen) in space.dma_pieces(va, len)? {
        let size = if by_range { plen } else { 0 };
        if let Err(e) = v.unmap_one(h, pva, size, flags)
            && first.is_ok()
        {
            first = Err(e);
        }
    }
    first
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
    /// ★ What RM stamped at birth, read from the alloc reply by [`birth_privilege`] — `Some` only
    /// for a channel [`HostRm::birth_member`] created (so `born_user` USER-checked it). `None` for
    /// a value assembled elsewhere (a test literal), which [`Channel::assert_user`] refuses.
    pub born_user: Option<BirthPrivilege>,
}

/// Why [`Channel::assert_user`] refused a channel.
pub const CHANNEL_NOT_BORN_USER: &str =
    "the host channel carries no USER birth stamp from the birth path";
/// Why [`Channel::assert_user`] refused a channel whose stamp reads privileged.
pub const CHANNEL_STAMP_PRIVILEGED: &str =
    "the host channel's birth reply has PRIVILEGED_CHANNEL set";

impl Channel {
    /// ★★★ Owner ruling 2026-10-07 (§S item 1): guest-kernel work beyond copy-engine scrubbing
    /// (the kernel-GR tier) runs only on a host channel that is verifiably USER. `Ok` with RM's
    /// stamp when the channel came out of the birth path (`crate::birth::born_user`: the alloc ran
    /// with `CAP_SYS_ADMIN` cleared and the reply's `PRIVILEGED_CHANNEL` and `internalFlags`
    /// level were read back as USER) and the stamp still reads USER; refused by name otherwise.
    ///
    /// # Errors
    /// [`CHANNEL_NOT_BORN_USER`] or [`CHANNEL_STAMP_PRIVILEGED`].
    pub fn assert_user(&self) -> Result<BirthPrivilege, &'static str> {
        let p = self.born_user.ok_or(CHANNEL_NOT_BORN_USER)?;
        if p.reply_flags & NVOS04_FLAGS_PRIVILEGED_CHANNEL_TRUE != 0 {
            return Err(CHANNEL_STAMP_PRIVILEGED);
        }
        Ok(p)
    }
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

/// ★ NEGATIVE CONTROL — `KF3_NEGCTL_SKIP_CAP_BRACKET=1` issues the channel-alloc call WITHOUT
/// clearing `CAP_SYS_ADMIN`. In a VMM that holds the capability, RM then stamps the channel
/// `ADMIN` and the reply check must refuse **every** birth: the knob can only make births fail,
/// never let a privileged channel live. It exists to show the check fires on a real RM reply
/// (a check that has only ever reported zero has not shown it can report one).
fn negctl_skip_cap_bracket() -> bool {
    static SKIP: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SKIP.get_or_init(|| {
        let on = std::env::var_os("KF3_NEGCTL_SKIP_CAP_BRACKET").is_some_and(|v| v == "1");
        if on {
            eprintln!(
                "kf-host: ⚠ NEGATIVE CONTROL KF3_NEGCTL_SKIP_CAP_BRACKET=1: channel-alloc calls \
                 keep CAP_SYS_ADMIN; every birth RM stamps privileged must be refused"
            );
        }
        on
    })
}

/// ★ The `NV_CHANNEL_ALLOC_PARAMS` request every kf3 channel birth sends.
///
/// `flags` asks for exactly one thing, `DENY_PHYSICAL_MODE_CE` (P6b: an operand that escaped the
/// rewriter must not reach host physical memory, `ogkm-580 alloc_channel.h:158-170`), and leaves
/// `PRIVILEGED_CHANNEL` (5:5) **clear**. That clear request is what makes the reply check exact:
/// RM never clears a requested bit 5 on a USER channel and always sets it on ADMIN or KERNEL
/// (`kernel_channel.c:278-290`), so a set bit in the reply can only be RM's own verdict.
#[must_use]
pub fn channel_alloc_request(ring: &RingSpec, engine_type: u32) -> ChannelAllocParams {
    channel_alloc_request_with(ring, engine_type, true)
}

/// ★ Review 2026-10-08 (finding 2): the `DENY_PHYSICAL_MODE_CE` belt as a TYPE, so a call site cannot
/// swap it with the neighbouring `first` boolean and silently turn the belt off on production channels.
/// Production always passes [`PhysicalCeBelt::Deny`]; `Off` is for the physical-operand oracle only
/// (`kf-harness kf-phys-oracle`), and a test pins that no other crate names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicalCeBelt {
    /// `DENY_PHYSICAL_MODE_CE = TRUE` (every production channel).
    Deny,
    /// The belt off (oracle only; the channel is still born and checked `USER`).
    Off,
}

impl PhysicalCeBelt {
    /// The flag value.
    #[must_use]
    pub const fn denied(self) -> bool {
        matches!(self, Self::Deny)
    }
}

/// Like [`channel_alloc_request`] but with `DENY_PHYSICAL_MODE_CE` chosen by the caller.
///
/// ⊘ **Production always passes `deny_physical_ce = true`** ([`channel_alloc_request`]). The
/// `false` form exists for ONE caller: the physical-operand oracle (`kf-harness kf-phys-oracle`),
/// which exercises whether the hardware honours a physical CE operand on a channel that is
/// unprivileged **but without kayfabe's own CE-deny belt** — isolating the belt's contribution
/// from the channel's `PRIVILEGE_USER` level (`OWNER_RULINGS.md` §U.3; the owner's Windows
/// channel-policy question). It never relaxes the `PRIVILEGED_CHANNEL` (5:5) clear, so the born
/// channel's `USER` level is still checked in the alloc reply, whichever value this flag takes.
#[must_use]
pub fn channel_alloc_request_with(
    ring: &RingSpec,
    engine_type: u32,
    deny_physical_ce: bool,
) -> ChannelAllocParams {
    ChannelAllocParams {
        h_object_error: ring.err_notifier,
        gp_fifo_offset: ring.gp_fifo_va,
        gp_fifo_entries: ring.gp_fifo_entries,
        // ★ P6b: DENY physical-mode CE on EVERY channel we birth (Translated rings and
        // Passthrough twins alike) — `NVOS04_FLAGS_CHANNEL_DENY_PHYSICAL_MODE_CE` 7:7
        // (`ogkm-580 alloc_channel.h:158-170`: "regardless of whether or not the client handle
        // is admin"). No operand we author is physical (the rewriter turns them into window
        // VAs), so this only ever stops one that ESCAPED the rewriter from reaching host
        // physical memory: `[measured p6b8]` UVM's CE launches on subchannel 4 escaped a
        // subchannel-keyed rewriter, verbatim and physical, with no Xid (fixed at commit
        // 00f62991).
        flags: if deny_physical_ce {
            NVOS04_FLAGS_CHANNEL_DENY_PHYSICAL_MODE_CE_TRUE
        } else {
            0
        },
        h_context_share: 0,
        h_va_space: 0,
        h_userd_memory_0: ring.userd_memory,
        userd_offset_0: ring.userd_offset,
        engine_type,
    }
}

/// What RM stamped on a channel kf3 births, read from the alloc **reply**.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BirthPrivilege {
    /// The reply's `flags` word (`PRIVILEGED_CHANNEL` clear).
    pub reply_flags: u32,
}

/// Why a birth's privilege is refused — each one by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivilegeRefusal {
    /// The REQUEST asked for `PRIVILEGED_CHANNEL`. Then a set bit in the reply would be our own
    /// request echoed back and the readback would prove nothing, so the request itself is refused.
    RequestedPrivileged,
    /// The reply is too short to hold `flags`.
    ReplyUnreadable,
    /// RM set `PRIVILEGED_CHANNEL` in the reply: the channel is `ADMIN` or `KERNEL`
    /// (`kernel_channel.c:278-287` sets the bit on both; the reply does not say which).
    PrivilegedChannel {
        /// The reply's `flags` word.
        reply_flags: u32,
    },
    /// The reply's `internalFlags` `PRIVILEGE` field (1:0) names a level other than `USER`.
    /// ⊘ On 580 RM zeroes `internalFlags` before it copies the reply out
    /// (`kernel_channel.c:1056-1058`), so this reads `USER` there and bit 5 is the load-bearing
    /// reading; the field is checked so a driver that does return it is held to it too.
    NotUser {
        /// The `PRIVILEGE` field: 1 `ADMIN`, 2 `KERNEL`, 3 undefined.
        level: u32,
    },
}

impl core::fmt::Display for PrivilegeRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PrivilegeRefusal::RequestedPrivileged => {
                write!(f, "the request asked for PRIVILEGED_CHANNEL")
            }
            PrivilegeRefusal::ReplyUnreadable => write!(f, "the reply holds no flags word"),
            PrivilegeRefusal::PrivilegedChannel { reply_flags } => write!(
                f,
                "RM stamped PRIVILEGED_CHANNEL=1 (reply flags {reply_flags:#010x}): an ADMIN or \
                 KERNEL channel"
            ),
            PrivilegeRefusal::NotUser { level } => {
                write!(f, "the reply's privilege level is {level}, not USER (0)")
            }
        }
    }
}

/// ★★★ The per-birth tripwire: is the channel RM just created a `USER` channel?
///
/// `requested_flags` is the `flags` word kf3 sent; `reply` is the `NV_CHANNEL_ALLOC_PARAMS` image
/// RM copied back (bench layout — `HostRm::raw_alloc` carries a different host layout back to it).
///
/// # Errors
/// [`PrivilegeRefusal`], by name, for anything but a readable `USER` reply to a request that did
/// not ask for privilege.
pub fn birth_privilege(
    requested_flags: u32,
    reply: &[u8],
) -> Result<BirthPrivilege, PrivilegeRefusal> {
    if requested_flags & NVOS04_FLAGS_PRIVILEGED_CHANNEL_TRUE != 0 {
        return Err(PrivilegeRefusal::RequestedPrivileged);
    }
    let reply_flags = reply
        .get(CHANNEL_ALLOC_FLAGS_OFF..CHANNEL_ALLOC_FLAGS_OFF + 4)
        .and_then(|b| b.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or(PrivilegeRefusal::ReplyUnreadable)?;
    if reply_flags & NVOS04_FLAGS_PRIVILEGED_CHANNEL_TRUE != 0 {
        return Err(PrivilegeRefusal::PrivilegedChannel { reply_flags });
    }
    if let Ok(Some(p)) = kf_abi::notifier::ChannelNotifierWire::V580.decode_privilege(reply)
        && p.level != kf_abi::notifier::ChannelPrivilege::USER
    {
        return Err(PrivilegeRefusal::NotUser { level: p.level });
    }
    Ok(BirthPrivilege { reply_flags })
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
        self.alloc_vaspace_with(std::env::var_os("KF3_NO_GUEST_VA_RESERVE").is_none())
    }

    /// ★ P1+P2 inc B (`docs/design/V3_P1P2_TSPACE.md` §2.1): a fresh host VA space with NONE of the
    /// [`GUEST_VA_RANGES`] reservations — the Translated space (T-space), where no guest row ever
    /// lands. Its caller reserves what it needs ([`HostRm::reserve_va`]) and may record one
    /// reservation in [`VaSpace::guest`] so FIXED maps inside it go through it and
    /// [`HostRm::free_vaspace`] frees it.
    ///
    /// # Errors
    /// The host's refusal; a half-built space is freed.
    pub fn alloc_vaspace_bare(&self) -> Result<VaSpace, RmError> {
        self.alloc_vaspace_with(false)
    }

    fn alloc_vaspace_with(&self, reserve_guest: bool) -> Result<VaSpace, RmError> {
        let mut params = [0u8; NvVaspaceAllocationParameters::SIZE];
        // ★ 2026-10-08 (task A): a guest-mirror space starts where the guest's page tables can map
        // ([`TWIN_VA_FLOOR`] states the rule and its evidence). ⊘ Replaces the run70-72 diagnostic
        // `KF3_TWIN_VA_BASE=<hex>` (a guest-independent but operator-chosen base, with NO reservation
        // keeping host RM's own buffers out of the low range it opened).
        let start = if reserve_guest {
            MirrorVaStart::from_env()
        } else {
            MirrorVaStart::HostDefault
        };
        NvVaspaceAllocationParameters {
            va_base: start.va_base(),
            ..NvVaspaceAllocationParameters::default()
        }
        .encode_into(&mut params)
        .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        let want = self.mint();
        let space = self.raw_alloc(
            self.device,
            want,
            VA_SPACE,
            Some(kf_abi::hostabi::HostParams::Measured(
                &kf_abi::generated::matrix::NV_VASPACE_ALLOCATION_PARAMETERS,
            )),
            &mut params,
        )?;
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
        match self.raw_alloc(
            self.device,
            want,
            NV01_MEMORY_VIRTUAL,
            Some(kf_abi::hostabi::HostParams::Measured(
                &kf_abi::generated::matrix::NV_MEMORY_VIRTUAL_ALLOCATION_PARAMS,
            )),
            &mut range,
        ) {
            Ok(h) => {
                self.remember(h, self.device);
                let mut vas = VaSpace {
                    space,
                    range: h,
                    guest: [GuestVaRange::default(); 3],
                };
                // ★ v3-gfx: reserve the guest's ranges BEFORE anything is placed in the space. A
                // refusal leaves that range unreserved (the pre-v3-gfx behaviour), and says so.
                // ★ 2026-10-08: plus the low guest range of a floor-started space (slot 2).
                if reserve_guest {
                    let ranges = GUEST_VA_RANGES.iter().copied().chain(start.low_range());
                    for (slot, (lo, hi)) in vas.guest.iter_mut().zip(ranges) {
                        match self.reserve_va(space, lo, hi - lo) {
                            Ok(handle) => *slot = GuestVaRange { handle, lo, hi },
                            Err(e) => eprintln!(
                                "kf-host: space {space:#x}: guest VA range [{lo:#x}, {hi:#x}) NOT reserved: {e:?} — host RM may place its own objects there"
                            ),
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
        NvMemoryAllocationParams {
            owner: self.client.raw(),
            kind: 0,
            flags: 0,
            attr2: 0,
            attr: 0,
            size: len,
            alignment: 0,
        }
        .encode_into(&mut p)
        .map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        p[8..12].copy_from_slice(&NVOS32_RESERVE_FLAGS.to_le_bytes());
        p[80..88].copy_from_slice(&at.to_le_bytes()); // offset
        p[108..112].copy_from_slice(&space.to_le_bytes()); // hVASpace
        let want = self.mint();
        let h = self.raw_alloc(
            self.device,
            want,
            NV50_MEMORY_VIRTUAL,
            Some(kf_abi::hostabi::HostParams::Measured(
                &kf_abi::generated::matrix::NV_MEMORY_ALLOCATION_PARAMS,
            )),
            &mut p,
        )?;
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
        self.map_kind(
            space,
            memory,
            backing,
            offset,
            len,
            at,
            defer,
            0,
            MapPerm::READ_WRITE,
        )
    }

    /// ★ v3-gfx: [`HostRm::map`] with a PTE `kind` (0 = PITCH, no override). A non-zero kind is
    /// set with `NVOS46_FLAGS_PAGE_KIND_OVERRIDE_YES` + `kindOverride` (`nvos.h:2113-2115, 2177`),
    /// which host RM validates with `FB_IS_KIND_SUPPORTED` (`virtual_mem.c:1348-1357`). The caller
    /// passes an UNCOMPRESSED kind; this device backs no comptags.
    ///
    /// ★★★ v3-roperm: `perm` is the guest leaf's permissions, carried to the host PTE
    /// ([`MapPerm::nvos46_flags`]). A read-only guest mapping is read-only on the host, so a GPU
    /// write through it FAULTS on the twin instead of landing.
    ///
    /// ★ **STATUS (2026-10-09, LIVE): a FIXED row that straddles a reservation edge is SPLIT, not
    /// refused.** `[measured, run 225]` Windows maps `0xb0000+0x80000` and `0xff000+0x2000`, across
    /// the 1 MiB edge of the low reservation; the refusal (`VA_STRADDLES_RESERVATION`) left the VA
    /// unmapped and the GPU faulted on it (Xid 31). A FIXED `NV_ESC_RM_MAP_MEMORY_DMA` names one
    /// `hDma`, so the row becomes one map per piece of [`VaSpace::dma_pieces`], each of the SAME
    /// memory object at `offset + (piece.va - at)`. ALL OR NOTHING: if a piece is refused, the pieces
    /// already mapped are unmapped again (with this call's `defer`) and that piece's error is
    /// returned. Each piece asserts its own placement ([`HostRm::raw_map_dma_slice`]).
    ///
    /// # Errors
    /// As [`HostRm::map`]; plus [`VA_BELOW_TWIN_FLOOR`], [`VA_ROW_WRAPS`], [`VA_PIECE_UNPLACEABLE`]
    /// before any host call.
    #[allow(clippy::too_many_arguments)]
    pub fn map_kind(
        &self,
        space: VaSpace,
        memory: u32,
        backing: MapBacking,
        offset: u64,
        len: u64,
        at: Option<u64>,
        defer: bool,
        kind: u8,
        perm: MapPerm,
    ) -> Result<u64, RmError> {
        let extra = if defer {
            NVOS46_FLAGS_DEFER_TLB_INVALIDATION_TRUE
        } else {
            0
        };
        let extra = extra
            | if kind != 0 {
                NVOS46_FLAGS_PAGE_KIND_OVERRIDE_YES
            } else {
                0
            };
        let extra = extra | perm.nvos46_flags();
        let shared = backing == MapBacking::SharedSlice;
        match at {
            // ★ 2026-10-09: a FIXED map is one map per `hDma` it touches ([`map_pieces`]).
            Some(a) => map_pieces(
                self,
                space,
                MapPiece {
                    memory,
                    offset,
                    len,
                    at: a,
                    extra,
                    shared,
                    kind: u32::from(kind),
                },
                unmap_flags(defer),
            ),
            None => self.raw_map_dma_slice(
                space.range,
                memory,
                offset,
                len,
                None,
                extra,
                shared,
                u32::from(kind),
            ),
        }
    }

    /// ★ Map ALL of `memory` (`len` bytes) into `space` at an address RM chooses, and return it —
    /// a **window**: the identity window over the store (guest FB-physical `p` ⇒ `base + p`) or
    /// the guest-RAM window. ⊘ The base is READ BACK, never assumed (`THE_TRANSLATED_PLANE.md`
    /// §16.1, §17.1): RM chose `0x120000000` on GA106/580, and a later driver may not.
    /// `high` maps it GROWS_DOWN, away from a guest kernel's bottom-up allocations (§24.2).
    ///
    /// # Errors
    /// The host's refusal.
    pub fn map_window(
        &self,
        space: VaSpace,
        memory: u32,
        len: u64,
        high: bool,
    ) -> Result<u64, RmError> {
        self.map_window_perm(space, memory, len, high, MapPerm::READ_WRITE)
    }

    /// ★ P1+P2 inc B: [`HostRm::map_window`] with the mapping's permissions — the T-space maps its
    /// guest-RAM window GPU-uncached ([`MapPerm::volatile`], `V3_P1P2_TSPACE.md` §13) so a
    /// CPU-written sysmem semaphore is never served from L2.
    ///
    /// # Errors
    /// The host's refusal.
    pub fn map_window_perm(
        &self,
        space: VaSpace,
        memory: u32,
        len: u64,
        high: bool,
        perm: MapPerm,
    ) -> Result<u64, RmError> {
        self.map_window_paged(space, memory, len, high, perm, 0)
    }

    /// ★ P1+P2 review fix (2026-10-04): [`HostRm::map_window_perm`] with a page-size pin
    /// (`page_size`: an `NVOS46_FLAGS_PAGE_SIZE_*` value, 0 = RM chooses). The T-space pins its
    /// store window to 2 MiB pages so RM never rounds the map past its length
    /// (`kf_abi::bringup::NVOS46_FLAGS_PAGE_SIZE_HUGE`).
    ///
    /// # Errors
    /// The host's refusal.
    pub fn map_window_paged(
        &self,
        space: VaSpace,
        memory: u32,
        len: u64,
        high: bool,
        perm: MapPerm,
        page_size: u32,
    ) -> Result<u64, RmError> {
        let extra = if high {
            NVOS46_FLAGS_DMA_OFFSET_GROWS_DOWN
        } else {
            0
        } | perm.nvos46_flags()
            | page_size;
        self.raw_map_dma_slice(space.range, memory, 0, len, None, extra, false, 0)
    }

    /// Unmap the mapping at `va` in `space`; `defer` as for [`HostRm::map`].
    ///
    /// # Errors
    /// The host's status.
    pub fn unmap(&self, space: VaSpace, va: u64, defer: bool) -> Result<(), RmError> {
        self.raw_unmap_dma_flags(space.dma_for(va, 1)?, va, unmap_flags(defer))
    }

    /// ★ 2026-10-09: unmap the ONE row `[va, va+len)` that a [`HostRm::map`] placed — every piece of
    /// it ([`VaSpace::dma_pieces`]), each by its exact start (the whole-mapping unmap, which every
    /// host driver carries). A row inside one object is exactly [`HostRm::unmap`]. Every piece is
    /// attempted even if one is refused; the first refusal is returned. `len` must be the length the
    /// row was MAPPED with (a smaller one leaves the later pieces mapped; use
    /// [`HostRm::unmap_range`] for a sub-range).
    ///
    /// # Errors
    /// The host's status for the first refused piece; [`VA_BELOW_TWIN_FLOOR`], [`VA_ROW_WRAPS`],
    /// [`VA_PIECE_UNPLACEABLE`] before any host call.
    pub fn unmap_row(&self, space: VaSpace, va: u64, len: u64, defer: bool) -> Result<(), RmError> {
        unmap_pieces(self, space, va, len, false, unmap_flags(defer))
    }

    /// ★★★ Unmap EVERY mapping of ours in `space` that intersects `[va, va+len)` — one host call
    /// for any number of placements (`V3_BATCHED_MAP.md` §4; [`HostRm::raw_unmap_dma_range`]). A
    /// placement straddling an edge is split and keeps its outside part. `defer` as for
    /// [`HostRm::map`].
    ///
    /// ⊘ The range must be one the caller OWNS whole: RM removes whatever of this client's
    /// mappings lie in it, so a range reaching into a window or a ring would take it down too.
    ///
    /// ⊘★ **(2026-10-09) A range that SPLITS a mapping is safe ONLY inside a guest reservation.**
    /// [`VaSpace::range`] is an `NV01_MEMORY_VIRTUAL` (`bReserveVaOnAlloc = NV_FALSE`,
    /// `ogkm-595.84 virtual_mem.c:349`): each map through it allocates its own VA block, and an
    /// unmap frees the block CONTAINING the unmapped part's start (`virt_mem_allocator_gm107.c:1635
    /// -1638` → `gpu_vaspace.c:1631-1640`, a containment search, `eheap_old.c:1005-1027`) — so a
    /// partial unmap there frees the WHOLE original block and the remnants RM still lists lose
    /// their PTEs. `[measured, Windows runs 242/243]` host Xid 31 `FAULT_PTE` at `0x4034000` and
    /// `NV_ASSERT(NULL != pMemBlock) @ gpu_vaspace.c:1639` at exit. A reservation
    /// (`NV50_MEMORY_VIRTUAL`, [`GuestVaRange`]) invalidates exactly the unmapped PTEs
    /// (`virt_mem_allocator_gm107.c:1578-1633`). The caller (`kf_mem::batch::BatchedVas`) enforces
    /// it: [`VaSpace::guest_reserved`] is the predicate.
    ///
    /// ★ STATUS (2026-10-09): a range that straddles a reservation edge is no longer refused
    /// (`VA_STRADDLES_RESERVATION`, before any host call); it is unmapped piece by piece
    /// ([`VaSpace::dma_pieces`]), one range unmap per `hDma`. Every piece is attempted even if one
    /// is refused; the first refusal is returned.
    ///
    /// # Errors
    /// The host's status for the first refused piece; [`VA_BELOW_TWIN_FLOOR`], [`VA_ROW_WRAPS`],
    /// [`VA_PIECE_UNPLACEABLE`] before any host call.
    pub fn unmap_range(
        &self,
        space: VaSpace,
        va: u64,
        len: u64,
        defer: bool,
    ) -> Result<(), RmError> {
        if len == 0 {
            return Err(RmError::NoMemory);
        }
        unmap_pieces(self, space, va, len, true, unmap_flags(defer))
    }

    /// ★ 2026-10-09: ONE FIXED map of `[offset, offset+len)` of `memory` at `at` THROUGH `h_dma`
    /// — a reservation the caller made over that VA ([`HostRm::reserve_va`]; `kf_mem::batch`'s
    /// micro reservations). No piece routing: the caller guarantees `[at, at+len)` lies inside
    /// `h_dma` (RM asserts it, `dma.c:155-159`). Flags as [`HostRm::map_kind`].
    ///
    /// # Errors
    /// The host's refusal, or [`RmError::PlacementRefused`].
    #[allow(clippy::too_many_arguments)]
    pub fn map_in(
        &self,
        h_dma: u32,
        memory: u32,
        backing: MapBacking,
        offset: u64,
        len: u64,
        at: u64,
        defer: bool,
        kind: u8,
        perm: MapPerm,
    ) -> Result<u64, RmError> {
        guest_row_end(at, len)?;
        let extra = if defer {
            NVOS46_FLAGS_DEFER_TLB_INVALIDATION_TRUE
        } else {
            0
        } | if kind != 0 {
            NVOS46_FLAGS_PAGE_KIND_OVERRIDE_YES
        } else {
            0
        } | perm.nvos46_flags();
        self.raw_map_dma_slice(
            h_dma,
            memory,
            offset,
            len,
            Some(at),
            extra,
            backing == MapBacking::SharedSlice,
            u32::from(kind),
        )
    }

    /// ★ 2026-10-09: ONE unmap THROUGH `h_dma` (a reservation of ours): `size == 0` the whole
    /// mapping keyed by its exact start, else every mapping intersecting `[va, va+size)` — exact
    /// inside an `NV50_MEMORY_VIRTUAL` reservation (`virt_mem_allocator_gm107.c:1578-1633`).
    ///
    /// # Errors
    /// The host's status.
    pub fn unmap_in(&self, h_dma: u32, va: u64, size: u64, defer: bool) -> Result<(), RmError> {
        self.raw_unmap_dma_range(h_dma, va, size, unmap_flags(defer))
    }

    /// ★★★ **Map N scattered pieces of a file at ONE VA-contiguous range, in O(1) host RM calls**
    /// (`V3_BATCHED_MAP.md` §3): stitch the pieces into one host view
    /// ([`kf_linux_raw::MappedRegion::stitch`]), describe it with ONE
    /// `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` (RM pins the pages and keeps no address — the view is
    /// dropped before this returns), then ONE fixed `NV_ESC_RM_MAP_MEMORY_DMA` of the whole object
    /// at `at`. Returns the object's handle: the caller owns it and frees it (with [`HostRm::free`])
    /// once no mapping of it is left — freeing it earlier would unmap every piece
    /// (`rs_client.c:1342-1395`, `_clientUnmapInterBackRefMappings`).
    ///
    /// ★ All or nothing: `Ok` ⇔ the host placed every piece at its VA; on any refusal nothing of
    /// ours is left (a failed map is rolled back by RM, `virt_mem_allocator_gm107.c:1540-1557`, a
    /// misplaced one torn down by [`HostRm::map`]'s placement assertion, and the object is freed).
    /// Mapped as a [`MapBacking::SharedSlice`] (4 KiB pinned): a stitched object is not physically
    /// contiguous at any bigger page.
    ///
    /// ★ STATUS (2026-10-09): the map is [`HostRm::map_kind`]'s, so a batch whose VA range straddles
    /// a reservation edge is mapped as one map per `hDma` of the ONE stitched object (all or
    /// nothing, rolled back by `map_kind`) — it is no longer refused with `VA_STRADDLES_RESERVATION`.
    ///
    /// # Errors
    /// The stitch (by name), the descriptor or the map — whichever refused.
    #[allow(clippy::too_many_arguments)]
    pub fn map_scattered(
        &self,
        space: VaSpace,
        fd: std::os::fd::BorrowedFd<'_>,
        pieces: &[(u64, u64)],
        at: u64,
        defer: bool,
        kind: u8,
        perm: MapPerm,
    ) -> Result<u32, ScatterError> {
        self.map_scattered_through(space, None, fd, pieces, at, defer, kind, perm)
    }

    /// ★ 2026-10-09: [`HostRm::map_scattered`] mapped THROUGH `through` (a reservation the caller
    /// made with [`HostRm::reserve_va`] over the batch's VA, `kf_mem::batch` "micro reservation")
    /// instead of the space's own routing — so a later partial unmap is exact (see
    /// [`HostRm::unmap_range`]). `None` is [`HostRm::map_scattered`].
    ///
    /// # Errors
    /// As [`HostRm::map_scattered`].
    #[allow(clippy::too_many_arguments)]
    pub fn map_scattered_through(
        &self,
        space: VaSpace,
        through: Option<u32>,
        fd: std::os::fd::BorrowedFd<'_>,
        pieces: &[(u64, u64)],
        at: u64,
        defer: bool,
        kind: u8,
        perm: MapPerm,
    ) -> Result<u32, ScatterError> {
        // ★ Review fix 2026-10-10: never stitch a new view while the reaper is behind — the
        // caller's thread (the VA manager) must not wait for it, and the views must not pile up.
        let backlog = view_reaper().map_or(0, |r| {
            r.flush();
            r.backlog()
        });
        if backlog >= REAP_OVERFLOW {
            return Err(ScatterError::ReaperBacklog(backlog));
        }
        let t0 = std::time::Instant::now();
        let view = kf_linux_raw::MappedRegion::stitch(
            fd,
            pieces,
            kf_linux_raw::HostProt::ReadWrite,
            kf_linux_raw::CachePolicy::WriteBack,
            kf_linux_raw::HostPageSize::query(),
        )
        .map_err(ScatterError::Stitch)?;
        let t_stitch = t0.elapsed();
        let len = view.len_bytes();
        let obj = self
            .alloc_os_descriptor(&view, kf_linux_raw::HostOffset::new(0), len)
            .map_err(ScatterError::Descriptor)?;
        let t_desc = t0.elapsed();
        // RM holds the pages now (and never the address): the view goes before any map exists —
        // to the reaper, because its `munmap` is the costliest step (`[measured bm3]` 80-97 ms for
        // 4 096 populated VMAs on the nested bench, vs 0.5 ms for the map itself).
        reap_view(view);
        let t_drop = t0.elapsed();
        let r = match through {
            None => self.map_kind(
                space,
                obj,
                MapBacking::SharedSlice,
                0,
                len,
                Some(at),
                defer,
                kind,
                perm,
            ),
            Some(h) => self.map_in(
                h,
                obj,
                MapBacking::SharedSlice,
                0,
                len,
                at,
                defer,
                kind,
                perm,
            ),
        };
        // ★ Bounded phase breakdown (the first 32 batches of the process): stitch vs pin vs map.
        static LOGGED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if LOGGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 32 {
            eprintln!(
                "kf-host: map_scattered {} pieces {len:#x} bytes: stitch {} us, descriptor {} us, hand view to reaper {} us, map {} us",
                pieces.len(),
                t_stitch.as_micros(),
                (t_desc - t_stitch).as_micros(),
                (t_drop - t_desc).as_micros(),
                (t0.elapsed() - t_drop).as_micros()
            );
        }
        match r {
            Ok(_) => Ok(obj),
            Err(e) => {
                let _ = self.free(obj);
                Err(ScatterError::Map(e))
            }
        }
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
        self.birth_channel_with(space, engine_type, ring, PhysicalCeBelt::Deny)
    }

    /// Like [`HostRm::birth_channel`] but with `DENY_PHYSICAL_MODE_CE` chosen by the caller.
    ///
    /// ⊘ Production uses [`HostRm::birth_channel`] (`deny_physical_ce = true`). The `false` form is
    /// for the physical-operand oracle only (see [`channel_alloc_request_with`]); the reply check
    /// still proves the channel `USER`.
    ///
    /// # Errors
    /// As [`HostRm::birth_channel`].
    pub fn birth_channel_with(
        &self,
        space: VaSpace,
        engine_type: u32,
        ring: RingSpec,
        belt: PhysicalCeBelt,
    ) -> Result<Channel, RmError> {
        if !ring.userd_offset.is_multiple_of(USERD_ALIGNMENT) {
            return Err(RmError::Other(USERD_OFFSET_MISALIGNED));
        }
        let tsg = self.birth_group(space, engine_type)?;
        self.birth_member(tsg, engine_type, ring, true, belt)
            .inspect_err(|_| {
                let _ = self.free(tsg);
            })
    }

    /// ★ A host channel GROUP (`KEPLER_CHANNEL_GROUP_A`) over `space` on `engine_type`, with no
    /// member yet — the twin of ONE guest TSG, whose channels are born into it with
    /// [`HostRm::birth_member`]. `[measured vvid 2026-09-26]` CUDA puts its 8 GR channels in ONE
    /// TSG sharing ONE GR context; per-channel host TSGs split that context and a kernel launched
    /// on a second stream's channel fails the SKED local-memory check (Xid 13
    /// `SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE`) — context state pushed on one channel never reached it.
    ///
    /// # Errors
    /// The host's refusal.
    pub fn birth_group(&self, space: VaSpace, engine_type: u32) -> Result<u32, RmError> {
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
        let tsg = self.raw_alloc(
            self.device,
            want,
            CHANNEL_GROUP,
            Some(kf_abi::hostabi::HostParams::Measured(
                &kf_abi::generated::matrix::NV_CHANNEL_GROUP_ALLOCATION_PARAMETERS,
            )),
            &mut tsg_params,
        )?;
        self.remember(tsg, self.device);
        Ok(tsg)
    }

    /// ★ A channel over `ring` inside the host group `tsg` → bound → work-submit token. The FIRST
    /// member binds the group (`NVA06C_CTRL_CMD_BIND`, every member present); a later member binds
    /// itself (`NVA06F_CTRL_CMD_BIND` on the channel) so a bound group's other members are never
    /// re-bound. `hContextShare = 0`: the group's LEGACY subcontext, shared by every member
    /// (`kernel_channel.c:607-668`) — one GR context, as CUDA's one-ctxshare TSG has on hardware.
    /// A failure frees the channel (never the group, which the caller owns).
    ///
    /// `deny_physical_ce` is `DENY_PHYSICAL_MODE_CE`: production passes `true` on every channel
    /// (see [`channel_alloc_request`]); only the physical-operand oracle passes `false`, and the
    /// `born_user` reply check proves the channel `USER` either way.
    ///
    /// # Errors
    /// [`USERD_OFFSET_MISALIGNED`] before any host call; else the host's refusal.
    pub fn birth_member(
        &self,
        tsg: u32,
        engine_type: u32,
        ring: RingSpec,
        first: bool,
        belt: PhysicalCeBelt,
    ) -> Result<Channel, RmError> {
        if !ring.userd_offset.is_multiple_of(USERD_ALIGNMENT) {
            return Err(RmError::Other(USERD_OFFSET_MISALIGNED));
        }
        let request = channel_alloc_request_with(&ring, engine_type, belt.denied());
        let mut chan_params = [0u8; ChannelAllocParams::SIZE];
        if request.encode_into(&mut chan_params).is_err() {
            return Err(RmError::Other(ABI_ENCODE_FAILED));
        }
        let want = self.mint();
        let class = self.classes.gpfifo_channel().channel_id().0;
        // ★★★ THE ONLY WAY A CHANNEL CLASS REACHES HOST RM (`crate::birth`): the alloc runs with
        // `CAP_SYS_ADMIN` cleared from this thread's EFFECTIVE set, and RM's reply is checked
        // before the channel is used. RM stamps the channel's privilege from the calling thread's
        // `capable(CAP_SYS_ADMIN)` at this one ioctl (`ogkm-580: kernel_channel.c:277-291`,
        // `escape.c:304`), so the channel is `PRIVILEGE_USER` whatever the VMM runs as. The bit
        // stays permitted and is put back right after (`kf_linux_raw::capability`). Every other
        // alloc entry in this crate refuses the class (`birth::admit_alloc_class`). ⊘ Not the RM
        // client class: `NV01_ROOT_NON_PRIV` is rewritten to `NV01_ROOT_CLIENT` before RM sees it
        // (`escape.c:394-403`).
        let born = crate::birth::born_user(
            &kf_linux_raw::capability::ThisThread,
            negctl_skip_cap_bracket(),
            engine_type,
            request.flags,
            &mut chan_params,
            |inside, params| {
                let h = self.carried_alloc(
                    class,
                    Some(kf_abi::hostabi::HostParams::Measured(
                        &kf_abi::generated::matrix::NV_CHANNEL_ALLOC_PARAMS,
                    )),
                    params,
                    |p| self.raw_alloc_exact(tsg, want, class, p, Some(inside)),
                )?;
                self.remember(h, tsg);
                Ok(h)
            },
            |h| {
                let _ = self.free(h);
            },
        )?;
        let chan = born.handle;
        eprintln!(
            "kf-host: channel birth h={chan:#x} engine={engine_type:#x} reply_flags={:#010x} \
             PRIVILEGED_CHANNEL=0 privilege=USER cap_sys_admin={}",
            born.privilege.reply_flags, born.cap
        );
        let unwind = |me: &Self| {
            let _ = me.free(chan);
        };
        let mut bind = [0u8; BIND_PARAMS_SIZE];
        bind.copy_from_slice(&engine_type.to_le_bytes());
        let (on, cmd) = if first {
            (tsg, NVA06C_CTRL_CMD_BIND)
        } else {
            (chan, kf_abi::submit::NVA06F_CTRL_CMD_BIND)
        };
        if let Err(e) = self.raw_control(on, cmd, &mut bind) {
            unwind(self);
            return Err(e);
        }
        let mut token = [0u8; WORK_SUBMIT_TOKEN_PARAMS_SIZE];
        if let Err(e) = self.raw_control(
            chan,
            NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN,
            &mut token,
        ) {
            unwind(self);
            return Err(e);
        }
        Ok(Channel {
            tsg,
            chan,
            token: u32::from_le_bytes(token),
            born_user: Some(born.privilege),
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
        let h = self.raw_alloc(
            chan.chan,
            want,
            self.classes.ce_object().ce_object_id().0,
            Some(kf_abi::hostabi::HostParams::Measured(
                &kf_abi::generated::matrix::NVB0B5_ALLOCATION_PARAMETERS,
            )),
            &mut params,
        )?;
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
        let h = self.raw_alloc(
            chan.chan,
            want,
            class,
            Some(kf_abi::hostabi::HostParams::Measured(
                &kf_abi::generated::matrix::NV_GR_ALLOCATION_PARAMETERS,
            )),
            &mut params,
        )?;
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
    pub fn alloc_engine_object(
        &self,
        chan: Channel,
        class: u32,
        copy_engine: Option<u32>,
    ) -> Result<u32, RmError> {
        let mut ce = [0u8; CeAllocParams::SIZE];
        let mut gr = [0u8; 16];
        let params: &mut [u8] = match copy_engine {
            Some(engine_type) => {
                CeAllocParams {
                    version: CeAllocParams::VERSION_1,
                    engine_type,
                }
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
        let strukt = if copy_engine.is_some() {
            kf_abi::hostabi::HostParams::Measured(
                &kf_abi::generated::matrix::NVB0B5_ALLOCATION_PARAMETERS,
            )
        } else {
            kf_abi::hostabi::HostParams::Measured(
                &kf_abi::generated::matrix::NV_GR_ALLOCATION_PARAMETERS,
            )
        };
        let want = self.mint();
        let h = self.raw_alloc(chan.chan, want, class, Some(strukt), params)?;
        self.remember(h, chan.chan);
        Ok(h)
    }

    /// ★ A VIDEO engine object (NVENC / NVDEC / OFA class) of `class` on `chan`, with params WE author:
    /// `NV_MSENC_ALLOCATION_PARAMETERS` / `NV_BSP_ALLOCATION_PARAMETERS` — the same 12 bytes
    /// `{size = 12, prohibitMultipleInstances = 0, engineInstance}` (`ogkm-580: nvos.h:2943-2996`),
    /// OFA has the same three U32 fields (`nvos.h:3011-3016`), carried through
    /// the decoder ABI carrier (`kf-chan/src/passthrough.rs:188-193`).
    /// `engineInstance` = the twin's own engine index, so nothing of the guest's alloc but its
    /// class reaches the host. Host RM (a GSP client itself) allocates and promotes the falcon
    /// context (`kernel_falcon.c:279-299`) — the guest's own context buffer is never used.
    ///
    /// # Errors
    /// The host's refusal.
    pub fn alloc_video_object(
        &self,
        chan: Channel,
        class: u32,
        engine_instance: u32,
    ) -> Result<u32, RmError> {
        let mut p = [0u8; 12];
        p[0..4].copy_from_slice(&12u32.to_le_bytes());
        p[8..12].copy_from_slice(&engine_instance.to_le_bytes());
        // The two blocks are one 12-byte layout at every measured tag, renamed NVENC/NVDEC at
        // 610 — carried under the class's own name: `xxB7` is an encoder class, `xxB0` a decoder.
        let m = &kf_abi::generated::matrix::NV_MSENC_ALLOCATION_PARAMETERS;
        let strukt = if class & 0xff == 0xb7 {
            kf_abi::hostabi::HostParams::Renamed {
                before: m,
                after: &kf_abi::generated::matrix::NV_NVENC_ALLOCATION_PARAMETERS,
            }
        } else {
            kf_abi::hostabi::HostParams::Renamed {
                before: &kf_abi::generated::matrix::NV_BSP_ALLOCATION_PARAMETERS,
                after: &kf_abi::generated::matrix::NV_NVDEC_ALLOCATION_PARAMETERS,
            }
        };
        let want = self.mint();
        let h = self.raw_alloc(chan.chan, want, class, Some(strukt), &mut p)?;
        self.remember(h, chan.chan);
        Ok(h)
    }

    /// ⚠ EXPERIMENT (2026-10-08, `KF3_WIN_TWIN_DEFAPI_OBJECT`, default off): an `NV50_DEFERRED_API`
    /// (`0x5080`) under `chan`, with NO params (`RS_OPTIONAL(NV5080_ALLOC_PARAMS)`,
    /// `ogkm-595.84: resource_list.h:1524-1528`; `notifyCompletion` stays false) and nothing
    /// registered on it — the only thing it can do is be found by a `SET_OBJECT` on the channel's
    /// software subchannel. Unprivileged class (`RS_FLAGS_ALLOC_NON_PRIVILEGED`, OWNER_RULINGS §U.1).
    ///
    /// # Errors
    /// The host's refusal.
    pub fn alloc_deferred_api(&self, chan: Channel) -> Result<u32, RmError> {
        let want = self.mint();
        let h = self.raw_alloc(
            chan.chan,
            want,
            kf_abi::generated::classes::NV50_DEFERRED_API_CLASS,
            None,
            &mut [],
        )?;
        self.remember(h, chan.chan);
        Ok(h)
    }

    /// ★ EXPERIMENT `x11-dispsw` (default off; `docs/design/V3_DISPLAY.md`, the 2026-10-03 note): a
    /// `GF100_DISP_SW` object on `chan` — the host twin of a guest's display-software object, so the
    /// software methods the guest's channel sends reach a host object instead of raising Xid 32.
    /// Params WE author, never the guest's: [`DISP_SW_AUTHORED_PARAMS`] = `{logicalHeadId 0,
    /// displayMask 0, caps 0}` (`ogkm-580: class/cl9072.h`). Host RM's constructor needs a display
    /// engine (`disp_sw.c:67-71`) and head 0 < its head count (`:83-88`); `displayMask 0` skips the
    /// active-display check (`:90-98`). Unprivileged (`RS_FLAGS_ALLOC_NON_PRIVILEGED`,
    /// `resource_list.h:1502-1511`). A host GPU without a display engine refuses it here.
    ///
    /// # Errors
    /// The host's refusal.
    pub fn alloc_disp_sw(&self, chan: Channel) -> Result<u32, RmError> {
        let mut params = DISP_SW_AUTHORED_PARAMS;
        let want = self.mint();
        let h = self.raw_alloc(
            chan.chan,
            want,
            GF100_DISP_SW,
            Some(kf_abi::hostabi::HostParams::Measured(
                &kf_abi::generated::matrix::NV9072_ALLOCATION_PARAMETERS,
            )),
            &mut params,
        )?;
        self.remember(h, chan.chan);
        Ok(h)
    }

    /// ★ EXPERIMENT `x11-dispsw` (review 2026-10-03, MEDIUM): the FIFO **software classID** host RM
    /// gave `object`, a [`Self::alloc_disp_sw`] object on `chan` — the 16-bit value a
    /// `SET_OBJECT` must carry to name it. Host RM numbers every `ENG_SW` child of a channel from
    /// that channel's own counter (`kchannelRegisterChild`, `ogkm-580: kernel_channel.c:3408-3453`)
    /// and answers it here: `NV906F_CTRL_GET_CLASS_ENGINEID` on the channel
    /// (`ctrl906f.h:94-103`; `classEngineID = DRF_NUM(906F, _SET_OBJECT, _NVCLASS, classID)`,
    /// `kernel_channel_gm107.c:72-82`, `NVCLASS` = `15:0`, `cl906f.h:71`). Unprivileged
    /// (`NON_PRIVILEGED`, flags `0x10008`, `g_kernel_channel_nvoc.c:256-259`). The guest's client
    /// asks its OWN CPU-RM the same question about its own object (`kernel_channel.c:2950-2970`),
    /// so the twin serves its `SET_OBJECT` only when both numbers agree.
    ///
    /// ⊘ CORRECTED 2026-10-08 (run wl1, host 595.91.07): the paragraph below was the defect —
    /// outside `[580.65.06, 581)` the unlisted control was refused before the ioctl
    /// (`HOST_ABI_REFUSED`, logged as `Other(19314)`), so x11-dispsw refused every guest
    /// GF100_DISP_SW at 595 and the guest X driver failed "display software resources". It is now
    /// a measured `HOST_CONTROLS` row (`NV906F_CTRL_GET_CLASS_ENGINEID_PARAMS`, `host_chan_cmds`).
    /// As first written: Not a [`kf_abi::hostabi::HOST_CONTROLS`] row (the driver matrix has no
    /// `NV906F_CTRL_GET_CLASS_ENGINEID_PARAMS`): carried as bytes on the interval the host encoders
    /// were written for and refused by name elsewhere — a refusal the caller turns into a refused
    /// display-SW alloc.
    ///
    /// # Errors
    /// The host's refusal, or [`RmError::Other`] when the reply names another class than
    /// `GF100_DISP_SW` for `object`.
    pub fn disp_sw_class_id(&self, chan: Channel, object: u32) -> Result<u16, RmError> {
        let mut p = [0u8; GET_CLASS_ENGINEID_PARAMS_SIZE];
        p[0..4].copy_from_slice(&object.to_le_bytes());
        self.raw_control(chan.chan, NV906F_CTRL_GET_CLASS_ENGINEID, &mut p)?;
        decode_disp_sw_class_id(&p).ok_or(RmError::Other(DISP_SW_CLASS_ID_UNREADABLE))
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
        let h = self.raw_alloc(
            self.device,
            want,
            0x83de,
            Some(kf_abi::hostabi::HostParams::Measured(
                &kf_abi::generated::matrix::NV83DE_ALLOC_PARAMETERS,
            )),
            &mut params,
        )?;
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
    pub fn set_ctxsw_preemption_mode(
        &self,
        chan: Channel,
        flags: u32,
        gfxp: u32,
        cilp: u32,
    ) -> Result<(), RmError> {
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
    pub fn flush_gpu_cache(
        &self,
        aperture: u32,
        write_back: bool,
        invalidate: bool,
    ) -> Result<(), RmError> {
        const SIZE: usize = 4024;
        let flags = (aperture & 0x3)
            | (u32::from(write_back) << 2)
            | (u32::from(invalidate) << 3)
            | (1 << 4);
        let mut p = vec![0u8; SIZE];
        p[4016..4020].copy_from_slice(&flags.to_le_bytes());
        self.raw_control(self.subdevice, 0x2080_130e, &mut p)
    }

    /// ★ w828: the same verb with ONLY `FB_FLUSH_YES` (flags bit 5) — the host's sysmembar
    /// (`kmemsysFlushGpuCache_IMPL` → `kbusSendSysmembar`, `ogkm-580: src/nvidia/src/kernel/gpu/
    /// mem_sys/kern_mem_sys_ctrl.c:1470-1476`; *"If only the FB flush is needed, only the _APERTURE
    /// and _FB_FLUSH_YES are needed"*, `:1500-1502`). Serves a Hopper+ guest's
    /// `NV_XAL_EP_UFLUSH_FB_FLUSH` token read.
    ///
    /// # Errors
    /// The host's status.
    pub fn fb_flush(&self) -> Result<(), RmError> {
        const SIZE: usize = 4024;
        let mut p = vec![0u8; SIZE];
        p[4016..4020].copy_from_slice(&(1u32 << 5).to_le_bytes());
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
        let mut p = kf_abi::submit::Preempt {
            wait: true,
            manual_timeout: false,
            timeout_us: 0,
        }
        .encode();
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
    pub fn disable_channels(
        &self,
        chans: &[Channel],
        disable: bool,
        only_scheduling: bool,
        rewind_gp_put: bool,
    ) -> Result<(), RmError> {
        let d = kf_abi::submit::DisableChannels {
            disable,
            only_disable_scheduling: only_scheduling,
            rewind_gp_put,
            runlist_preempt_event: 0,
            list: chans.iter().map(|c| (self.client.raw(), c.chan)).collect(),
        };
        let mut p = d.encode().map_err(|_| RmError::Other(ABI_ENCODE_FAILED))?;
        self.raw_control(
            self.subdevice,
            kf_abi::submit::NV2080_CTRL_CMD_FIFO_DISABLE_CHANNELS,
            &mut p,
        )
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
        let limit = len
            .checked_sub(1)
            .ok_or(RmError::Other(ABI_ENCODE_FAILED))?;
        let mut p = [0u8; 32];
        p[4..8].copy_from_slice(&NVOS03_FLAGS_HASH_TABLE_DISABLE.to_le_bytes());
        p[8..12].copy_from_slice(&memory.to_le_bytes());
        p[16..24].copy_from_slice(&offset.to_le_bytes());
        p[24..32].copy_from_slice(&limit.to_le_bytes());
        let want = self.mint();
        let h = self.raw_alloc(
            self.device,
            want,
            NV01_CONTEXT_DMA,
            Some(kf_abi::hostabi::HostParams::Measured(
                &kf_abi::generated::matrix::NV_CONTEXT_DMA_ALLOCATION_PARAMS,
            )),
            &mut p,
        )?;
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

    /// ★ Free ONE member of a shared group (the channel only); the group is freed by its owner
    /// when its last member goes ([`HostRm::free`] on `chan.tsg`).
    ///
    /// # Errors
    /// The host's status.
    pub fn free_member(&self, chan: Channel) -> Result<(), RmError> {
        self.free(chan.chan)
    }
}

/// ★★★ **v3-roperm — THE PERMISSIONS A HOST GPU MAPPING CARRIES**, as the guest's leaf stated
/// them (`V3_UVM_DEMAND_PAGING.md` §6). Each field is one the host's UNPRIVILEGED map verb can
/// express (`NVOS46`), so a guest permission is never widened on the twin:
///
/// | field | NVOS46 | host PTE (VER2 / VER3 PCF) |
/// |---|---|---|
/// | `read_only` | `ACCESS_READ_ONLY` | `READ_ONLY` / `_RO_` |
/// | `atomic_disable` | `TLB_LOCK_ENABLE` | `ATOMIC_DISABLE` / `NO_ATOMIC` |
/// | `volatile` | `GPU_CACHEABLE_NO` | `VOL` / `UNCACHED` |
///
/// ⊘ `volatile: false` maps `GPU_CACHEABLE_DEFAULT`, never `_YES`: the host memory keeps its own
/// attribute, so the twin is never CACHED where the host object is not (the pre-roperm mapping).
/// ⊘ `PRIVILEGE` has no field: RM takes it from the memory descriptor
/// (`virt_mem_allocator_gm107.c:2849-2850`), not from a client's flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct MapPerm {
    /// GPU writes fault.
    pub read_only: bool,
    /// GPU atomics fault.
    pub atomic_disable: bool,
    /// The GPU does not cache the mapping.
    pub volatile: bool,
}

impl MapPerm {
    /// The pre-roperm mapping: read-write, atomics allowed, the memory's own cache attribute.
    pub const READ_WRITE: MapPerm = MapPerm {
        read_only: false,
        atomic_disable: false,
        volatile: false,
    };

    /// The `NVOS46_PARAMETERS::flags` bits that place these permissions.
    #[must_use]
    pub const fn nvos46_flags(self) -> u32 {
        (if self.read_only {
            NVOS46_FLAGS_ACCESS_READ_ONLY
        } else {
            0
        }) | (if self.atomic_disable {
            NVOS46_FLAGS_TLB_LOCK_ENABLE
        } else {
            0
        }) | (if self.volatile {
            NVOS46_FLAGS_GPU_CACHEABLE_NO
        } else {
            0
        })
    }
}

/// Why a [`HostRm::map_scattered`] placed nothing — each step named, so a refusal says whether the
/// host kernel, the descriptor or the GPU map refused.
#[derive(Debug)]
pub enum ScatterError {
    /// The stitched host view could not be built (e.g. a hugetlb backing, `max_map_count`).
    Stitch(kf_linux_raw::RawError),
    /// `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` refused.
    Descriptor(RmError),
    /// The fixed map refused (incl. `VA_ALREADY_MAPPED`, [`RmError::PlacementRefused`]).
    Map(RmError),
    /// ★ Review fix 2026-10-10: [`REAP_OVERFLOW`] stitched views already wait for the reaper —
    /// nothing was stitched (the caller maps the rows one by one; never a wait on the reaper).
    ReaperBacklog(usize),
}

/// ★ `V3_BATCHED_MAP.md` §3: release a stitched view OFF the caller's thread.
///
/// The view is dead the moment its descriptor exists — RM pinned its pages and keeps no address
/// (`os-mlock.c:216-254`, `nv.c:3357-3400`); nothing reads it — so WHEN it is unmapped changes
/// nothing but who waits. One long-lived reaper thread `munmap`s views in order. If the thread
/// cannot be started, the view is dropped here, as before.
///
/// ★ Review fix 2026-10-10 (rule: no thread that serves input may stall). ⊘ The hand-over used to
/// be a BLOCKING `SyncSender::send` into a queue of [`REAP_QUEUE`]: with the reaper behind (each
/// `munmap` of a 4 096-VMA view is 80-97 ms on the nested bench, §7.2) the VA-manager thread
/// waited for it. Now it never waits ([`Reaper::hand`]): a full queue parks the view in a bounded
/// overflow, retried on every later hand-over and before every stitch; [`HostRm::map_scattered`]
/// refuses to stitch while [`REAP_OVERFLOW`] views wait (the rows go per run, by name), so the
/// views waiting are bounded by `REAP_QUEUE + REAP_OVERFLOW` and none is ever forgotten.
fn reap_view(view: kf_linux_raw::MappedRegion) {
    match view_reaper() {
        Some(r) => {
            if let Err(v) = r.hand(view) {
                // Unreachable while `map_scattered` checks the backlog first (one producer); the
                // bound holds anyway: dropped here (an inline `munmap`), never queued unbounded.
                drop(v);
            }
        }
        None => drop(view),
    }
}

/// The process's view reaper (`None` if its thread could not be started).
fn view_reaper() -> Option<&'static Reaper<kf_linux_raw::MappedRegion>> {
    static REAPER: std::sync::OnceLock<Option<Reaper<kf_linux_raw::MappedRegion>>> =
        std::sync::OnceLock::new();
    REAPER
        .get_or_init(|| Reaper::spawn("kf-view-reaper", REAP_QUEUE, REAP_OVERFLOW, drop))
        .as_ref()
}

/// Stitched views that may wait for the reaper in its queue.
const REAP_QUEUE: usize = 2;
/// ★ Review fix 2026-10-10: views that may wait beside the queue (the hand-over never blocks).
pub const REAP_OVERFLOW: usize = 2;

/// ★ Review fix 2026-10-10 — **a hand-over to one worker thread that NEVER blocks the caller.**
/// `T` goes into a bounded queue (`try_send`); when the queue is full it waits in a bounded
/// overflow that every later [`Reaper::hand`] / [`Reaper::flush`] retries first, in order; past
/// the overflow bound the value is handed BACK ([`Reaper::hand`] → `Err`) for the caller to deal
/// with — never a wait, never an unbounded pile, never a dropped value nobody released.
pub struct Reaper<T: Send + 'static> {
    tx: std::sync::Mutex<std::sync::mpsc::SyncSender<T>>,
    overflow: std::sync::Mutex<std::collections::VecDeque<T>>,
    max_overflow: usize,
}

impl<T: Send + 'static> Reaper<T> {
    /// A worker thread named `name` that calls `work` on every value, in order; `None` if the
    /// thread cannot be started.
    pub fn spawn(
        name: &str,
        queue: usize,
        max_overflow: usize,
        work: impl Fn(T) + Send + 'static,
    ) -> Option<Self> {
        let (tx, rx) = std::sync::mpsc::sync_channel::<T>(queue);
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                for v in rx {
                    work(v);
                }
            })
            .ok()
            .map(|_| Self::over(tx, max_overflow))
    }

    /// A reaper over an existing sender (the worker is the caller's).
    #[must_use]
    pub fn over(tx: std::sync::mpsc::SyncSender<T>, max_overflow: usize) -> Self {
        Reaper {
            tx: std::sync::Mutex::new(tx),
            overflow: std::sync::Mutex::new(std::collections::VecDeque::new()),
            max_overflow,
        }
    }

    /// Hand `v` to the worker without ever blocking.
    ///
    /// # Errors
    /// `v` back when the queue is full and the overflow already holds its bound (or the worker is
    /// gone): the caller releases it itself.
    pub fn hand(&self, v: T) -> Result<(), T> {
        self.flush();
        let Ok(mut over) = self.overflow.lock() else {
            return Err(v);
        };
        if !over.is_empty() {
            if over.len() >= self.max_overflow {
                return Err(v);
            }
            over.push_back(v);
            return Ok(());
        }
        let sent = match self.tx.lock() {
            Ok(tx) => tx.try_send(v),
            Err(_) => return Err(v),
        };
        match sent {
            Ok(()) => Ok(()),
            Err(std::sync::mpsc::TrySendError::Full(v)) if self.max_overflow > 0 => {
                over.push_back(v);
                Ok(())
            }
            Err(
                std::sync::mpsc::TrySendError::Full(v)
                | std::sync::mpsc::TrySendError::Disconnected(v),
            ) => Err(v),
        }
    }

    /// Move what waits in the overflow into the queue, in order, as far as it has room (never
    /// blocking).
    pub fn flush(&self) {
        let (Ok(mut over), Ok(tx)) = (self.overflow.lock(), self.tx.lock()) else {
            return;
        };
        while let Some(v) = over.pop_front() {
            match tx.try_send(v) {
                Ok(()) => {}
                Err(
                    std::sync::mpsc::TrySendError::Full(v)
                    | std::sync::mpsc::TrySendError::Disconnected(v),
                ) => {
                    over.push_front(v);
                    break;
                }
            }
        }
    }

    /// Values waiting in the overflow (not yet in the queue).
    #[must_use]
    pub fn backlog(&self) -> usize {
        self.overflow.lock().map_or(usize::MAX, |o| o.len())
    }
}

/// `GF100_DISP_SW` (`ogkm-580: class/cl9072.h`).
pub const GF100_DISP_SW: u32 = 0x9072;

/// `NV906F_CTRL_GET_CLASS_ENGINEID` (`ogkm-580: ctrl906f.h:94`) — on a channel, for one of its
/// children: `{hObject, classEngineID, classID, engineID}` (`:98-103`).
pub const NV906F_CTRL_GET_CLASS_ENGINEID: u32 = 0x906f_0101;
/// `sizeof(NV906F_CTRL_GET_CLASS_ENGINEID_PARAMS)` — four 32-bit words.
const GET_CLASS_ENGINEID_PARAMS_SIZE: usize = 16;

/// ★ x11-dispsw: the software classID out of an `NV906F_CTRL_GET_CLASS_ENGINEID` reply for a
/// `GF100_DISP_SW` object — `classEngineID`'s `NVCLASS` field (`15:0`, `cl906f.h:71`; for an
/// `ENG_SW` child it is the per-channel classID, `kernel_channel_gm107.c:72-82`). `None` when the
/// reply's `classID` (the object's external class, `RES_GET_EXT_CLASS_ID`, `:69`) is not
/// `GF100_DISP_SW` or the block is short: then the number does not name OUR object.
#[must_use]
pub fn decode_disp_sw_class_id(reply: &[u8]) -> Option<u16> {
    let w = |i: usize| {
        reply
            .get(4 * i..4 * i + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let class_engine = w(1)?;
    (w(2)? == GF100_DISP_SW).then_some((class_engine & 0xffff) as u16)
}

/// ★ EXPERIMENT `x11-dispsw`: the ONLY `NV9072_ALLOCATION_PARAMETERS` kayfabe sends its host —
/// `{logicalHeadId 0, displayMask 0, caps 0}`, a constant: [`HostRm::alloc_disp_sw`] takes no guest
/// input at all (owner rule: author host flags, never forward them).
pub const DISP_SW_AUTHORED_PARAMS: [u8; 12] = [0; 12];

#[cfg(test)]
mod disp_sw_tests {
    /// ★ x11-dispsw: the software classID is `classEngineID`'s low 16 bits, read only when the
    /// reply's `classID` is `GF100_DISP_SW` — a reply about another class (or a short block) names
    /// nothing we can compare, and is refused rather than read as a number.
    #[test]
    fn the_class_id_is_read_from_nvclass_only_for_a_disp_sw_reply() {
        let reply = |class_engine: u32, class: u32| {
            let mut p = [0u8; super::GET_CLASS_ENGINEID_PARAMS_SIZE];
            p[0..4].copy_from_slice(&0xcafe_0040u32.to_le_bytes());
            p[4..8].copy_from_slice(&class_engine.to_le_bytes());
            p[8..12].copy_from_slice(&class.to_le_bytes());
            p
        };
        assert_eq!(super::decode_disp_sw_class_id(&reply(3, 0x9072)), Some(3));
        // ENGINE (20:16) is zero for a software class; only NVCLASS is the number.
        assert_eq!(
            super::decode_disp_sw_class_id(&reply(0x001f_0007, 0x9072)),
            Some(7)
        );
        assert_eq!(super::decode_disp_sw_class_id(&reply(3, 0xc797)), None);
        assert_eq!(super::decode_disp_sw_class_id(&reply(3, 0x9072)[..8]), None);
    }

    /// ★ The authored display-SW params have the driver matrix's `NV9072_ALLOCATION_PARAMETERS`
    /// layout at every tag it covers (one 12-byte layout, the three words at 0/4/8; the sweep of
    /// 2026-10-03, `traces/driver_matrix/ranges.tsv`), and each word is zero: head 0, no display
    /// mask, no caps.
    #[test]
    fn the_authored_disp_sw_params_are_head_0_mask_0_caps_0_at_every_tag() {
        let s = &kf_abi::generated::matrix::NV9072_ALLOCATION_PARAMETERS;
        for &v in kf_abi::generated::matrix::MEASURED {
            let l = s.at(v).expect("measured").expect("present at every tag");
            assert_eq!(l.size(), super::DISP_SW_AUTHORED_PARAMS.len(), "{v:?}");
            for (name, off) in [("logicalHeadId", 0), ("displayMask", 4), ("caps", 8)] {
                let f = l.field(name).expect("consumed field");
                assert_eq!((f.off, f.size), (off, 4), "{name} at {v:?}");
                let at = off as usize;
                assert_eq!(
                    u32::from_le_bytes(
                        super::DISP_SW_AUTHORED_PARAMS[at..at + 4]
                            .try_into()
                            .expect("4 bytes")
                    ),
                    0,
                    "{name}"
                );
            }
        }
    }
}

#[cfg(test)]
mod twin_va_start_tests {
    use super::{
        GUEST_VA_RANGES, GuestVaRange, HOST_DEFAULT_VA_START, MirrorVaStart, TWIN_VA_FLOOR,
        VA_BELOW_TWIN_FLOOR, VA_ROW_WRAPS, VA_STRADDLES_RESERVATION, VaSpace, guest_row_end,
    };
    use crate::RmError;

    fn floor_space() -> VaSpace {
        let (lo, hi) = MirrorVaStart::GuestFloor.low_range().expect("low range");
        let mut guest = [GuestVaRange::default(); 3];
        for (i, (l, h)) in GUEST_VA_RANGES
            .iter()
            .copied()
            .chain([(lo, hi)])
            .enumerate()
        {
            guest[i] = GuestVaRange {
                handle: 0x100 + u32::try_from(i).expect("small"),
                lo: l,
                hi: h,
            };
        }
        VaSpace {
            space: 1,
            range: 2,
            guest,
        }
    }

    /// ★ Task A: the rule is OFF by default (Linux keeps host RM's start, nothing reserved low), and
    /// ON it starts at one 64 KiB big page with `[64 KiB, 1 MiB)` reserved — constants, not inputs.
    #[test]
    fn the_start_is_host_default_unless_windows_user_twins_are_enabled() {
        assert_eq!(MirrorVaStart::select(false), MirrorVaStart::HostDefault);
        assert_eq!(MirrorVaStart::HostDefault.va_base(), 0);
        assert_eq!(MirrorVaStart::HostDefault.low_range(), None);
        assert_eq!(MirrorVaStart::select(true), MirrorVaStart::GuestFloor);
        assert_eq!(MirrorVaStart::GuestFloor.va_base(), 0x1_0000);
        assert_eq!(
            MirrorVaStart::GuestFloor.low_range(),
            Some((TWIN_VA_FLOOR, HOST_DEFAULT_VA_START))
        );
        // The low range lies wholly below every other guest range: host RM's own placements keep
        // the room they had with the default start.
        let (_, hi) = MirrorVaStart::GuestFloor.low_range().expect("low");
        assert!(GUEST_VA_RANGES.iter().all(|&(lo, _)| hi <= lo));
    }

    /// ★ Task A: the rows Windows mapped (`[measured, runs 69-72 at d67e9290..8de8ef26, 2026-10-08]`) resolve — `0x10000+0x6000`
    /// (the compositor's low page) and `0x13000` through the low reservation, `0x1_2000_2000` through
    /// the first guest range, `0x400_0000` (a D3D ring) through the space's range.
    #[test]
    fn windows_low_rows_map_through_the_low_reservation() {
        let s = floor_space();
        assert_eq!(s.dma_for(0x1_0000, 0x6000), Ok(0x102));
        assert_eq!(s.dma_for(0x1_3000, 0x1000), Ok(0x102));
        assert_eq!(s.dma_for(0x1_2000_2000, 0x1_0000), Ok(0x100));
        assert_eq!(s.dma_for(0x400_0000, 0x1_0000), Ok(2));
        assert!(s.guest_reserved(0x1_0000, 0xf_0000));
    }

    /// The two refusal codes collide with no other kf-host code (`Other(n)` is how a refusal is
    /// named in the logs; run73 showed `0x4B72` already meant `HOST_ABI_REFUSED`).
    #[test]
    fn the_refusal_codes_are_unique_in_kf_host() {
        let codes = [
            crate::MAPPING_ATTRIBUTE_REFUSED,
            crate::VA_ALREADY_MAPPED,
            crate::HOST_ABI_REFUSED,
            crate::PRIVILEGED_CHANNEL_REFUSED,
            crate::CAP_BRACKET_REFUSED,
            crate::CHANNEL_CLASS_OUTSIDE_BIRTH,
            crate::DISP_SW_CLASS_ID_UNREADABLE,
            super::USERD_OFFSET_MISALIGNED,
            VA_STRADDLES_RESERVATION,
            VA_BELOW_TWIN_FLOOR,
            VA_ROW_WRAPS,
            super::VA_PIECE_UNPLACEABLE,
        ];
        let mut sorted = codes.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), codes.len());
    }

    /// ★ STATUS (2026-10-09): this pins [`VaSpace::dma_for`], the ONE-handle form, only. The map and
    /// unmap verbs no longer refuse a straddling row; they split it (`split_row_tests`).
    ///
    /// ★ Hostile input: a row in the NULL big page (VA 0 included), a row whose end wraps, and a row
    /// straddling the low reservation's edge are each refused BY NAME before any host call — a
    /// guest's page tables can never widen or move the twin's space.
    #[test]
    fn null_page_wrapped_and_straddling_rows_are_refused_by_name() {
        let s = floor_space();
        for va in [0, 0x1000, 0xf000] {
            assert_eq!(
                s.dma_for(va, 0x1000),
                Err(RmError::Other(VA_BELOW_TWIN_FLOOR)),
                "{va:#x}"
            );
        }
        assert_eq!(
            s.dma_for(u64::MAX - 0xfff, 0x2000),
            Err(RmError::Other(VA_ROW_WRAPS))
        );
        assert_eq!(
            guest_row_end(0x1_0000, u64::MAX),
            Err(RmError::Other(VA_ROW_WRAPS))
        );
        assert_eq!(
            s.dma_for(0xf_f000, 0x2000),
            Err(RmError::Other(VA_STRADDLES_RESERVATION))
        );
        // A zero-length row is one byte (the pre-existing convention).
        assert_eq!(guest_row_end(0x1_0000, 0), Ok(0x1_0001));
    }
}

/// ★ 2026-10-09 — a guest row that straddles a reservation edge is SPLIT into one piece per `hDma`
/// (`[measured, run 225]` Windows' `0xb0000+0x80000` and `0xff000+0x2000` across the 1 MiB edge of the
/// low reservation; the old refusal let the GPU fault on the unmapped VA, Xid 31).
#[cfg(test)]
mod split_row_tests {
    use super::{
        DmaVerbs, GUEST_VA_RANGES, GuestVaRange, HOST_DEFAULT_VA_START, HOST_HOLE_LO, MapPiece,
        TWIN_VA_FLOOR, VA_BELOW_TWIN_FLOOR, VA_PIECE_UNPLACEABLE, VA_PIECES_MAX, VA_ROW_WRAPS,
        VaSpace, map_pieces, unmap_pieces,
    };
    use crate::RmError;
    use std::cell::RefCell;

    const LOW: u32 = 0x102;
    const RANGE: u32 = 2;

    /// The production layout of a Windows twin: slots 0-1 the guest ranges, slot 2 the low range.
    fn floor_space() -> VaSpace {
        let mut guest = [GuestVaRange::default(); 3];
        for (i, (l, h)) in GUEST_VA_RANGES
            .iter()
            .copied()
            .chain([(TWIN_VA_FLOOR, HOST_DEFAULT_VA_START)])
            .enumerate()
        {
            guest[i] = GuestVaRange {
                handle: 0x100 + u32::try_from(i).expect("small"),
                lo: l,
                hi: h,
            };
        }
        VaSpace {
            space: 1,
            range: RANGE,
            guest,
        }
    }

    fn custom(res: &[(u32, u64, u64)]) -> VaSpace {
        let mut guest = [GuestVaRange::default(); 3];
        for (slot, &(handle, lo, hi)) in guest.iter_mut().zip(res) {
            *slot = GuestVaRange { handle, lo, hi };
        }
        VaSpace {
            space: 1,
            range: RANGE,
            guest,
        }
    }

    fn named(code: u32) -> Result<Vec<(u32, u64, u64)>, RmError> {
        Err(RmError::Other(code))
    }

    /// ★ 2026-10-09: the batch predicate (`kf_mem::batch::SpaceVerbs::splits_safely`) — only a
    /// range wholly inside ONE guest reservation may hold a mapping a range unmap will split; the
    /// Windows process VAs `[1 MiB, 4.5 GiB)` (`[measured run 243]` `0x4034000`) and anything
    /// crossing an edge go through the `NV01` range, where a split frees the whole VA block.
    #[test]
    fn only_a_range_inside_one_reservation_splits_safely() {
        let s = floor_space();
        let (lo, hi) = GUEST_VA_RANGES[0];
        assert!(s.guest_reserved(lo, 0x4000));
        assert!(
            s.guest_reserved(0x2_0000_0000, 0x10_0000),
            "CUDA VAs (gate 4: 128 GiB)"
        );
        assert!(
            !s.guest_reserved(0x403_0000, 0x8000),
            "Windows process VA: the NV01 range"
        );
        assert!(!s.guest_reserved(lo - 0x1000, 0x2000), "crossing an edge");
        assert!(
            !s.guest_reserved(hi - 0x1000, 0x2000),
            "crossing into the host hole"
        );
        assert!(!s.guest_reserved(HOST_HOLE_LO, 0x1000));
        assert!(
            s.guest_reserved(TWIN_VA_FLOOR, 0x1000),
            "the Windows low range is reserved"
        );
    }

    #[test]
    fn a_row_inside_one_reservation_is_one_piece() {
        let s = floor_space();
        assert_eq!(
            s.dma_pieces(0x1_0000, 0x6000),
            Ok(vec![(LOW, 0x1_0000, 0x6000)])
        );
        assert_eq!(
            s.dma_pieces(0x1_2000_2000, 0x1_0000),
            Ok(vec![(0x100, 0x1_2000_2000, 0x1_0000)])
        );
        assert_eq!(
            s.dma_pieces(0x400_0000, 0x1_0000),
            Ok(vec![(RANGE, 0x400_0000, 0x1_0000)])
        );
        // The whole low reservation, exactly.
        assert_eq!(
            s.dma_pieces(TWIN_VA_FLOOR, HOST_DEFAULT_VA_START - TWIN_VA_FLOOR),
            Ok(vec![(
                LOW,
                TWIN_VA_FLOOR,
                HOST_DEFAULT_VA_START - TWIN_VA_FLOOR
            )])
        );
    }

    /// ★ The two rows run 225 measured Windows mapping.
    #[test]
    fn the_rows_windows_mapped_across_the_1_mib_edge_split_in_two() {
        let s = floor_space();
        assert_eq!(
            s.dma_pieces(0xb_0000, 0x8_0000),
            Ok(vec![
                (LOW, 0xb_0000, 0x5_0000),
                (RANGE, 0x10_0000, 0x3_0000)
            ])
        );
        assert_eq!(
            s.dma_pieces(0xf_f000, 0x2000),
            Ok(vec![(LOW, 0xf_f000, 0x1000), (RANGE, 0x10_0000, 0x1000)])
        );
        // One byte either side of the edge.
        assert_eq!(
            s.dma_pieces(0xf_ffff, 2),
            Ok(vec![(LOW, 0xf_ffff, 1), (RANGE, 0x10_0000, 1)])
        );
    }

    #[test]
    fn a_row_ending_or_starting_exactly_on_an_edge_is_one_piece() {
        let s = floor_space();
        assert_eq!(
            s.dma_pieces(0xf_0000, 0x1_0000),
            Ok(vec![(LOW, 0xf_0000, 0x1_0000)]),
            "ends exactly on the edge"
        );
        assert_eq!(
            s.dma_pieces(0x10_0000, 0x1000),
            Ok(vec![(RANGE, 0x10_0000, 0x1000)]),
            "starts exactly on the edge"
        );
        // Upper edge of a high reservation: [1 TiB, 2^47).
        assert_eq!(
            s.dma_pieces((1 << 47) - 0x1000, 0x1000),
            Ok(vec![(0x101, (1 << 47) - 0x1000, 0x1000)])
        );
        assert_eq!(
            s.dma_pieces((1 << 47) - 0x1000, 0x2000),
            Ok(vec![
                (0x101, (1 << 47) - 0x1000, 0x1000),
                (RANGE, 1 << 47, 0x1000)
            ])
        );
    }

    #[test]
    fn below_the_floor_wrap_and_zero_length_keep_their_names() {
        let s = floor_space();
        for va in [0, 0x1000, 0xf000, 0xffff] {
            assert_eq!(
                s.dma_pieces(va, 0x1000),
                named(VA_BELOW_TWIN_FLOOR),
                "{va:#x}"
            );
        }
        // A row starting below the floor and reaching past it is still refused whole.
        assert_eq!(s.dma_pieces(0xf000, 0x10_0000), named(VA_BELOW_TWIN_FLOOR));
        assert_eq!(s.dma_pieces(u64::MAX - 0xfff, 0x2000), named(VA_ROW_WRAPS));
        assert_eq!(s.dma_pieces(0x1_0000, u64::MAX), named(VA_ROW_WRAPS));
        assert_eq!(s.dma_pieces(u64::MAX, u64::MAX), named(VA_ROW_WRAPS));
        assert_eq!(s.dma_pieces(u64::MAX, 1), named(VA_ROW_WRAPS));
        // A zero-length row is the one-byte convention, as ONE zero-length piece.
        assert_eq!(s.dma_pieces(0xf_ffff, 0), Ok(vec![(LOW, 0xf_ffff, 0)]));
    }

    #[test]
    fn a_row_spanning_a_reservation_entirely_has_three_pieces() {
        let s = floor_space();
        // The range below 4.5 GiB, ALL of the first guest range, then the host hole's first page.
        let (lo, hi) = GUEST_VA_RANGES[0];
        let va = lo - 0x2000;
        let len = (HOST_HOLE_LO + 0x1000) - va;
        assert_eq!(hi, HOST_HOLE_LO);
        assert_eq!(
            s.dma_pieces(va, len),
            Ok(vec![
                (RANGE, va, 0x2000),
                (0x100, lo, hi - lo),
                (RANGE, HOST_HOLE_LO, 0x1000)
            ])
        );
        let c = custom(&[(0x10, 0x20_000, 0x30_000)]);
        assert_eq!(
            c.dma_pieces(0x10_000, 0x30_000),
            Ok(vec![
                (RANGE, 0x10_000, 0x10_000),
                (0x10, 0x20_000, 0x10_000),
                (RANGE, 0x30_000, 0x10_000)
            ])
        );
    }

    #[test]
    fn two_adjacent_reservations_split_at_their_shared_edge() {
        let s = custom(&[(0x10, 0x1_0000, 0x2_0000), (0x11, 0x2_0000, 0x3_0000)]);
        assert_eq!(
            s.dma_pieces(0x1_f000, 0x2000),
            Ok(vec![(0x10, 0x1_f000, 0x1000), (0x11, 0x2_0000, 0x1000)])
        );
        assert_eq!(
            s.dma_pieces(0x1_0000, 0x2_0000),
            Ok(vec![(0x10, 0x1_0000, 0x1_0000), (0x11, 0x2_0000, 0x1_0000)])
        );
        // Out of order in the array: same answer.
        let r = custom(&[(0x11, 0x2_0000, 0x3_0000), (0x10, 0x1_0000, 0x2_0000)]);
        assert_eq!(
            r.dma_pieces(0x1_f000, 0x2000),
            s.dma_pieces(0x1_f000, 0x2000)
        );
    }

    #[test]
    fn an_unreserved_slot_is_the_range_and_an_absent_range_is_refused_by_name() {
        // Slot handle 0 = "not reserved": rows there go through the range, with no split.
        let s = custom(&[(0, 0x1_0000, 0x10_0000)]);
        assert_eq!(
            s.dma_pieces(0xb_0000, 0x8_0000),
            Ok(vec![(RANGE, 0xb_0000, 0x8_0000)])
        );
        // A space with no range object cannot place the outside piece, and says so.
        let mut z = floor_space();
        z.range = 0;
        assert_eq!(
            z.dma_pieces(0xb_0000, 0x8_0000),
            named(VA_PIECE_UNPLACEABLE)
        );
        assert_eq!(
            z.dma_pieces(0xb_0000, 0x1000),
            Ok(vec![(LOW, 0xb_0000, 0x1000)])
        );
    }

    #[test]
    fn the_piece_count_is_bounded_by_the_reservations() {
        // Three reservations separated by gaps, a row over all of them: 4 gaps + 3 = 7 pieces.
        let s = custom(&[
            (0x10, 0x2_0000, 0x3_0000),
            (0x11, 0x4_0000, 0x5_0000),
            (0x12, 0x6_0000, 0x7_0000),
        ]);
        let p = s.dma_pieces(0x1_0000, 0x7_0000).expect("pieces");
        assert_eq!(p.len(), VA_PIECES_MAX);
        assert_eq!(p.iter().map(|x| x.2).sum::<u64>(), 0x7_0000);
    }

    /// Deterministic xorshift, so the property is reproducible.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    /// ★ Property: for ANY (va, len), `dma_pieces` either names a refusal or returns pieces that
    /// start at `va`, are contiguous and in order, sum to exactly `len`, never exceed the row, number
    /// at most [`VA_PIECES_MAX`], and each lies wholly inside the reservation it names or wholly
    /// outside every reservation (then names the range). A one-piece answer is `dma_for`'s.
    #[test]
    fn pieces_always_partition_the_row() {
        let spaces = [
            floor_space(),
            custom(&[
                (0x10, 0x2_0000, 0x3_0000),
                (0x11, 0x3_0000, 0x5_0000),
                (0x12, 0x9_0000, 0xa_0000),
            ]),
            custom(&[]),
        ];
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        // Edges of every space, plus extremes: rows are placed around them.
        let mut anchors: Vec<u64> = vec![0, 1, 0xffff, 0x1_0000, u64::MAX, u64::MAX - 1, 1 << 47];
        for s in &spaces {
            for g in s.guest.iter().filter(|g| g.handle != 0) {
                anchors.extend([g.lo, g.hi]);
            }
        }
        let mut ok = 0u32;
        let mut refused = 0u32;
        for s in &spaces {
            for _ in 0..20_000 {
                let a = anchors[(rng.next() % anchors.len() as u64) as usize];
                let va = match rng.next() % 3 {
                    0 => a,
                    1 => a.wrapping_sub(rng.next() % 0x3_0000),
                    _ => a.wrapping_add(rng.next() % 0x3_0000),
                };
                let len = match rng.next() % 5 {
                    0 => rng.next(),
                    1 => rng.next() % 0x10,
                    2 => rng.next() % 0x2_0000,
                    3 => u64::MAX - rng.next() % 0x1000,
                    _ => rng.next() % 0x1000_0000,
                };
                match s.dma_pieces(va, len) {
                    Err(RmError::Other(c)) => {
                        assert!(
                            [VA_BELOW_TWIN_FLOOR, VA_ROW_WRAPS, VA_PIECE_UNPLACEABLE].contains(&c),
                            "{va:#x}+{len:#x}: unexpected refusal {c:#x}"
                        );
                        refused += 1;
                    }
                    Err(e) => panic!("{va:#x}+{len:#x}: unexpected error {e:?}"),
                    Ok(p) => {
                        ok += 1;
                        assert!(!p.is_empty() && p.len() <= VA_PIECES_MAX);
                        assert!(va >= TWIN_VA_FLOOR && va.checked_add(len.max(1)).is_some());
                        let mut cur = va;
                        for &(h, pva, plen) in &p {
                            assert_eq!(pva, cur, "contiguous, in order");
                            let pend = pva.checked_add(plen).expect("no wrap");
                            let inside = s.guest.iter().find(|g| {
                                g.handle == h && g.handle != 0 && g.lo <= pva && pend <= g.hi
                            });
                            let outside = h == s.range
                                && s.guest
                                    .iter()
                                    .filter(|g| g.handle != 0)
                                    .all(|g| pend <= g.lo || g.hi <= pva);
                            assert!(
                                inside.is_some() || outside,
                                "{va:#x}+{len:#x}: piece {h:#x} {pva:#x}+{plen:#x} is in no single object"
                            );
                            cur = pend;
                        }
                        let total: u64 = p.iter().map(|x| x.2).sum();
                        assert_eq!(total, if len == 0 { 0 } else { len });
                        if let [one] = p.as_slice()
                            && len != 0
                        {
                            assert_eq!(s.dma_for(va, len), Ok(one.0), "one piece is dma_for's");
                        }
                    }
                }
            }
        }
        assert!(
            ok > 1000 && refused > 1000,
            "both arms exercised: {ok}/{refused}"
        );
    }

    /// Records every host verb and refuses the n-th map / n-th unmap on request.
    #[derive(Default)]
    struct Fake {
        log: RefCell<Vec<String>>,
        maps: RefCell<u32>,
        unmaps: RefCell<u32>,
        fail_map_at: Option<u32>,
        fail_unmap_at: Option<u32>,
    }

    impl DmaVerbs for Fake {
        fn map_one(&self, h: u32, p: MapPiece) -> Result<u64, RmError> {
            let n = *self.maps.borrow();
            *self.maps.borrow_mut() += 1;
            if self.fail_map_at == Some(n) {
                self.log.borrow_mut().push(format!("map {h:#x} REFUSED"));
                return Err(RmError::Other(0x77));
            }
            self.log.borrow_mut().push(format!(
                "map {h:#x} mem {:#x} off {:#x} len {:#x} at {:#x} x{:#x} s{} k{}",
                p.memory, p.offset, p.len, p.at, p.extra, p.shared as u8, p.kind
            ));
            Ok(p.at)
        }
        fn unmap_one(&self, h: u32, va: u64, size: u64, flags: u32) -> Result<(), RmError> {
            let n = *self.unmaps.borrow();
            *self.unmaps.borrow_mut() += 1;
            self.log
                .borrow_mut()
                .push(format!("unmap {h:#x} {va:#x} size {size:#x} f{flags:#x}"));
            if self.fail_unmap_at == Some(n) {
                return Err(RmError::Other(0x78));
            }
            Ok(())
        }
    }

    fn row(offset: u64, len: u64, at: u64) -> MapPiece {
        MapPiece {
            memory: 0xAA,
            offset,
            len,
            at,
            extra: 0x40,
            shared: true,
            kind: 6,
        }
    }

    #[test]
    fn a_row_inside_one_object_is_one_unchanged_map() {
        let f = Fake::default();
        let s = floor_space();
        assert_eq!(
            map_pieces(&f, s, row(0x5000, 0x6000, 0x1_0000), 8),
            Ok(0x1_0000)
        );
        assert_eq!(
            *f.log.borrow(),
            ["map 0x102 mem 0xaa off 0x5000 len 0x6000 at 0x10000 x0x40 s1 k6"]
        );
    }

    #[test]
    fn a_straddling_map_issues_two_maps_of_the_same_object_at_the_right_offsets() {
        let f = Fake::default();
        let s = floor_space();
        assert_eq!(
            map_pieces(&f, s, row(0x5000, 0x8_0000, 0xb_0000), 8),
            Ok(0xb_0000)
        );
        assert_eq!(
            *f.log.borrow(),
            [
                "map 0x102 mem 0xaa off 0x5000 len 0x50000 at 0xb0000 x0x40 s1 k6",
                "map 0x2 mem 0xaa off 0x55000 len 0x30000 at 0x100000 x0x40 s1 k6",
            ]
        );
        let g = Fake::default();
        assert_eq!(map_pieces(&g, s, row(0, 0x2000, 0xf_f000), 8), Ok(0xf_f000));
        assert_eq!(
            *g.log.borrow(),
            [
                "map 0x102 mem 0xaa off 0x0 len 0x1000 at 0xff000 x0x40 s1 k6",
                "map 0x2 mem 0xaa off 0x1000 len 0x1000 at 0x100000 x0x40 s1 k6",
            ]
        );
    }

    #[test]
    fn a_refused_second_map_rolls_the_first_back() {
        let f = Fake {
            fail_map_at: Some(1),
            ..Fake::default()
        };
        let s = floor_space();
        assert_eq!(
            map_pieces(&f, s, row(0, 0x8_0000, 0xb_0000), 0x2),
            Err(RmError::Other(0x77)),
            "the refused piece's own error"
        );
        assert_eq!(
            *f.log.borrow(),
            [
                "map 0x102 mem 0xaa off 0x0 len 0x50000 at 0xb0000 x0x40 s1 k6",
                "map 0x2 REFUSED",
                // Exact-start whole-mapping unmap of the piece that landed, with the map's defer flag.
                "unmap 0x102 0xb0000 size 0x0 f0x2",
            ]
        );
    }

    #[test]
    fn a_refused_first_map_leaves_nothing_to_roll_back() {
        let f = Fake {
            fail_map_at: Some(0),
            ..Fake::default()
        };
        assert_eq!(
            map_pieces(&f, floor_space(), row(0, 0x8_0000, 0xb_0000), 0),
            Err(RmError::Other(0x77))
        );
        assert_eq!(*f.log.borrow(), ["map 0x102 REFUSED"]);
    }

    #[test]
    fn a_refused_third_piece_rolls_back_the_two_before_it_newest_first() {
        let s = custom(&[(0x10, 0x2_0000, 0x3_0000)]);
        let f = Fake {
            fail_map_at: Some(2),
            ..Fake::default()
        };
        assert_eq!(
            map_pieces(&f, s, row(0, 0x3_0000, 0x1_0000), 0),
            Err(RmError::Other(0x77))
        );
        let log = f.log.borrow();
        assert_eq!(log.len(), 5);
        assert_eq!(log[3], "unmap 0x10 0x20000 size 0x0 f0x0");
        assert_eq!(log[4], "unmap 0x2 0x10000 size 0x0 f0x0");
    }

    #[test]
    fn a_failing_rollback_still_returns_the_maps_error() {
        let f = Fake {
            fail_map_at: Some(1),
            fail_unmap_at: Some(0),
            ..Fake::default()
        };
        assert_eq!(
            map_pieces(&f, floor_space(), row(0, 0x8_0000, 0xb_0000), 0),
            Err(RmError::Other(0x77))
        );
    }

    #[test]
    fn hostile_rows_are_refused_before_any_host_call() {
        let f = Fake::default();
        let s = floor_space();
        for (r, code) in [
            (row(0, 0x1000, 0xf000), VA_BELOW_TWIN_FLOOR),
            (row(0, u64::MAX, 0x1_0000), VA_ROW_WRAPS),
            (row(0, 0x2000, u64::MAX - 0xfff), VA_ROW_WRAPS),
            // A memory offset that overflows once a straddling row is split.
            (row(u64::MAX - 0x10, 0x8_0000, 0xb_0000), VA_ROW_WRAPS),
        ] {
            assert_eq!(map_pieces(&f, s, r, 0), Err(RmError::Other(code)), "{r:?}");
        }
        assert_eq!(
            unmap_pieces(&f, s, 0xf000, 0x1000, false, 0),
            Err(RmError::Other(VA_BELOW_TWIN_FLOOR))
        );
        assert_eq!(
            unmap_pieces(&f, s, 0x1_0000, u64::MAX, true, 0),
            Err(RmError::Other(VA_ROW_WRAPS))
        );
        assert!(f.log.borrow().is_empty());
    }

    #[test]
    fn unmapping_a_straddling_row_unmaps_every_piece() {
        let s = floor_space();
        let f = Fake::default();
        assert_eq!(unmap_pieces(&f, s, 0xb_0000, 0x8_0000, false, 0x2), Ok(()));
        assert_eq!(
            *f.log.borrow(),
            [
                "unmap 0x102 0xb0000 size 0x0 f0x2",
                "unmap 0x2 0x100000 size 0x0 f0x2"
            ],
            "whole-mapping unmap keyed by each piece's start"
        );
        // A sub-range of the same placement: one range unmap per hDma, sized to the piece.
        let g = Fake::default();
        assert_eq!(unmap_pieces(&g, s, 0xf_f000, 0x2000, true, 0), Ok(()));
        assert_eq!(
            *g.log.borrow(),
            [
                "unmap 0x102 0xff000 size 0x1000 f0x0",
                "unmap 0x2 0x100000 size 0x1000 f0x0"
            ]
        );
        // A row inside one object stays one call.
        let h = Fake::default();
        assert_eq!(unmap_pieces(&h, s, 0x1_0000, 0x6000, true, 0), Ok(()));
        assert_eq!(*h.log.borrow(), ["unmap 0x102 0x10000 size 0x6000 f0x0"]);
    }

    #[test]
    fn a_refused_piece_unmap_does_not_stop_the_others_and_is_reported() {
        let f = Fake {
            fail_unmap_at: Some(0),
            ..Fake::default()
        };
        assert_eq!(
            unmap_pieces(&f, floor_space(), 0xb_0000, 0x8_0000, false, 0),
            Err(RmError::Other(0x78))
        );
        assert_eq!(
            f.log.borrow().len(),
            2,
            "the second piece was still unmapped"
        );
    }
}

#[cfg(test)]
mod perm_tests {
    use super::MapPerm;

    /// ★ v3-roperm: each permission sets exactly its own `NVOS46` field (`nvos.h:1974-1977`,
    /// `2107-2111`, `2129-2131`), and the default is the pre-roperm read-write map (no bits) —
    /// never `GPU_CACHEABLE_YES`, which would cache what the host object does not.
    #[test]
    fn each_permission_sets_exactly_its_nvos46_field() {
        assert_eq!(MapPerm::READ_WRITE.nvos46_flags(), 0);
        assert_eq!(MapPerm::default(), MapPerm::READ_WRITE);
        assert_eq!(
            MapPerm {
                read_only: true,
                ..MapPerm::READ_WRITE
            }
            .nvos46_flags(),
            0x1
        );
        assert_eq!(
            MapPerm {
                atomic_disable: true,
                ..MapPerm::READ_WRITE
            }
            .nvos46_flags(),
            1 << 28
        );
        assert_eq!(
            MapPerm {
                volatile: true,
                ..MapPerm::READ_WRITE
            }
            .nvos46_flags(),
            2 << 17
        );
        let all = MapPerm {
            read_only: true,
            atomic_disable: true,
            volatile: true,
        }
        .nvos46_flags();
        assert_eq!(all, 0x1 | (1 << 28) | (2 << 17));
        // None of them touches the fields the map path owns: FIXED 15, PAGE_SIZE 11:8, KIND_OVERRIDE
        // 19, DEFER 31, CACHE_SNOOP 4.
        assert_eq!(
            all & ((1 << 15) | (0xF << 8) | (1 << 19) | (1 << 31) | (1 << 4)),
            0
        );
    }

    /// ★★★★★ v3-adasys: EVERY map the crate makes asks RM to snoop the CPU cache
    /// (`NVOS46_FLAGS_CACHE_SNOOP_ENABLE`, field `4:4` = 1, `nvos.h:1992-1994`) — whatever the
    /// caller's permissions, kind, defer, grows-down, page-size pin or `FIXED` — and adding it
    /// changes no other bit. ⊘ The field's zero value is `_DISABLE`: without it a guest-RAM map
    /// is a `SYS_NONCOH` PTE, and on a platform that honours PCIe No-Snoop the copy engine reads
    /// stale DRAM (`[measured]` gates 3/4 FAIL 10/10 on a bare-metal Ryzen/B550, PASS 10/10 with it).
    #[test]
    fn every_map_snoops_the_cpu_cache_and_changes_nothing_else() {
        use kf_abi::bringup::{
            NVOS46_FLAGS_CACHE_SNOOP_ENABLE, NVOS46_FLAGS_DEFER_TLB_INVALIDATION_TRUE,
            NVOS46_FLAGS_DMA_OFFSET_FIXED_TRUE, NVOS46_FLAGS_DMA_OFFSET_GROWS_DOWN,
            NVOS46_FLAGS_PAGE_KIND_OVERRIDE_YES, NVOS46_FLAGS_PAGE_SIZE_4KB,
        };
        assert_eq!(
            NVOS46_FLAGS_CACHE_SNOOP_ENABLE, 0x10,
            "nvos.h: CACHE_SNOOP is 4:4, _ENABLE is 1"
        );
        let perms = [
            MapPerm::READ_WRITE,
            MapPerm {
                read_only: true,
                ..MapPerm::READ_WRITE
            },
            MapPerm {
                atomic_disable: true,
                volatile: true,
                read_only: true,
            },
        ];
        let others = [
            0,
            NVOS46_FLAGS_DEFER_TLB_INVALIDATION_TRUE,
            NVOS46_FLAGS_DMA_OFFSET_GROWS_DOWN,
            NVOS46_FLAGS_PAGE_KIND_OVERRIDE_YES | NVOS46_FLAGS_DEFER_TLB_INVALIDATION_TRUE,
        ];
        for perm in perms {
            for other in others {
                let extra = perm.nvos46_flags() | other;
                for page_size in [0, NVOS46_FLAGS_PAGE_SIZE_4KB] {
                    for fixed in [false, true] {
                        let f = crate::nvos46_map_flags(extra, page_size, fixed);
                        assert_ne!(
                            f & NVOS46_FLAGS_CACHE_SNOOP_ENABLE,
                            0,
                            "snoop missing: {f:#x}"
                        );
                        let want = extra
                            | page_size
                            | if fixed {
                                NVOS46_FLAGS_DMA_OFFSET_FIXED_TRUE
                            } else {
                                0
                            };
                        assert_eq!(
                            f & !NVOS46_FLAGS_CACHE_SNOOP_ENABLE,
                            want,
                            "another bit moved: {f:#x}"
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod privilege_tests {
    use super::{
        BirthPrivilege, NVOS04_FLAGS_CHANNEL_DENY_PHYSICAL_MODE_CE_TRUE,
        NVOS04_FLAGS_PRIVILEGED_CHANNEL_TRUE, PrivilegeRefusal, RingSpec, birth_privilege,
        channel_alloc_request,
    };
    use kf_abi::submit::ChannelAllocParams;

    fn ring() -> RingSpec {
        RingSpec {
            gp_fifo_va: 0x1_0000_0000,
            gp_fifo_entries: 512,
            userd_memory: 0xcafe_0001,
            userd_offset: 0x200,
            err_notifier: 0,
        }
    }

    /// The reply image RM would copy back for `request`, with `flags` as RM left it.
    fn reply(request: &ChannelAllocParams, flags: u32) -> [u8; ChannelAllocParams::SIZE] {
        let mut b = [0u8; ChannelAllocParams::SIZE];
        ChannelAllocParams { flags, ..*request }
            .encode_into(&mut b)
            .expect("encode");
        b
    }

    #[test]
    fn the_constants_are_the_header_fields() {
        // `ogkm-580: alloc_channel.h:141-143` (5:5) and `:168-170` (7:7).
        assert_eq!(NVOS04_FLAGS_PRIVILEGED_CHANNEL_TRUE, 0x20);
        assert_eq!(NVOS04_FLAGS_CHANNEL_DENY_PHYSICAL_MODE_CE_TRUE, 0x80);
    }

    /// ★ The request every birth sends: `PRIVILEGED_CHANNEL` clear (so the reply check is exact),
    /// `DENY_PHYSICAL_MODE_CE` set, nothing else in `flags`, and on the wire at +20.
    #[test]
    fn the_birth_request_never_asks_for_privilege() {
        for engine in [0x1, 0x9, 0xb, 0x13] {
            let req = channel_alloc_request(&ring(), engine);
            assert_eq!(req.flags & NVOS04_FLAGS_PRIVILEGED_CHANNEL_TRUE, 0);
            assert_eq!(req.flags, NVOS04_FLAGS_CHANNEL_DENY_PHYSICAL_MODE_CE_TRUE);
            let mut b = [0u8; ChannelAllocParams::SIZE];
            req.encode_into(&mut b).expect("encode");
            assert_eq!(&b[20..24], &0x80u32.to_le_bytes());
            assert_eq!(
                birth_privilege(req.flags, &b),
                Ok(BirthPrivilege { reply_flags: 0x80 })
            );
        }
    }

    /// ★★★ The tripwire: RM's verdict `PRIVILEGED_CHANNEL=1` refuses the birth by name.
    /// `0x004000a0` is the reply host RM wrote on an RTX 3060 at 580.159.04 on 2026-10-03 with the
    /// VMM holding CAP_SYS_ADMIN (`traces/v3_security/nonpriv_20261003/`, BEFORE; that branch also
    /// requested bit 22). `0xa0` is the same verdict on this request (bit 7), `0x20` bit 5 alone.
    #[test]
    fn a_privileged_reply_is_refused_by_name() {
        let req = channel_alloc_request(&ring(), 0x1);
        for flags in [0x0040_00a0, 0xa0, 0x20] {
            assert_eq!(
                birth_privilege(req.flags, &reply(&req, flags)),
                Err(PrivilegeRefusal::PrivilegedChannel { reply_flags: flags })
            );
        }
    }

    /// A USER reply passes whatever else RM set in `flags`, as long as bit 5 is clear.
    #[test]
    fn a_user_reply_passes_with_any_other_flag() {
        let req = channel_alloc_request(&ring(), 0x9);
        for flags in [0x80, 0x0040_0080, 0xffff_ffdf] {
            assert_eq!(
                birth_privilege(req.flags, &reply(&req, flags)),
                Ok(BirthPrivilege { reply_flags: flags })
            );
        }
    }

    /// ★★★ Owner ruling 2026-10-07 (§S item 1): the kernel-GR tier's assertion refuses a channel
    /// with no birth-path stamp, or whose stamp reads privileged, and admits a USER stamp.
    #[test]
    fn the_gr_tier_assertion_admits_only_a_user_birth_stamp() {
        let c = |born_user| super::Channel {
            tsg: 1,
            chan: 2,
            token: 3,
            born_user,
        };
        assert_eq!(c(None).assert_user(), Err(super::CHANNEL_NOT_BORN_USER));
        for flags in [0x20, 0xa0, 0x0040_00a0] {
            assert_eq!(
                c(Some(BirthPrivilege { reply_flags: flags })).assert_user(),
                Err(super::CHANNEL_STAMP_PRIVILEGED)
            );
        }
        let ok = BirthPrivilege { reply_flags: 0x80 };
        assert_eq!(c(Some(ok)).assert_user(), Ok(ok));
    }

    /// The reply's `internalFlags` PRIVILEGE field (1:0, at +244 on 580) must read USER too.
    #[test]
    fn a_non_user_internal_level_is_refused() {
        let req = channel_alloc_request(&ring(), 0x1);
        for level in [1u32, 2, 3] {
            let mut b = reply(&req, 0x80);
            b[244..248].copy_from_slice(&(level | (1 << 2)).to_le_bytes());
            assert_eq!(
                birth_privilege(req.flags, &b),
                Err(PrivilegeRefusal::NotUser { level })
            );
        }
        // The notifier-type bits (3:2) are not a privilege.
        let mut b = reply(&req, 0x80);
        b[244..248].copy_from_slice(&(2u32 << 2).to_le_bytes());
        assert!(birth_privilege(req.flags, &b).is_ok());
    }

    /// A request that itself asked for bit 5 makes the readback meaningless, so it is refused
    /// before the reply is even read; a reply too short to hold `flags` is refused, never read
    /// as zero.
    #[test]
    fn the_check_refuses_what_it_cannot_judge() {
        let req = channel_alloc_request(&ring(), 0x1);
        assert_eq!(
            birth_privilege(
                req.flags | NVOS04_FLAGS_PRIVILEGED_CHANNEL_TRUE,
                &reply(&req, 0x80)
            ),
            Err(PrivilegeRefusal::RequestedPrivileged)
        );
        assert_eq!(
            birth_privilege(req.flags, &[0u8; 23]),
            Err(PrivilegeRefusal::ReplyUnreadable)
        );
    }
}

#[cfg(test)]
mod reaper_tests {
    use super::Reaper;

    /// ★ Review fix 2026-10-10: the hand-over NEVER blocks — with the worker stalled (nobody
    /// drains the queue) every `hand` returns at once: the queue fills, then the bounded overflow,
    /// then the value comes BACK to the caller. Nothing is lost or reordered once the worker
    /// catches up. (At 6fafcc6e the hand-over was a blocking `send`: the second `hand` below would
    /// have waited for the worker forever.)
    #[test]
    fn a_stalled_reaper_never_blocks_the_caller_and_loses_nothing() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<u32>(1);
        let r = Reaper::over(tx, 2);
        assert_eq!(r.hand(1), Ok(()), "into the queue");
        assert_eq!(r.hand(2), Ok(()), "queue full: into the overflow");
        assert_eq!(r.hand(3), Ok(()));
        assert_eq!(r.backlog(), 2);
        assert_eq!(r.hand(4), Err(4), "the bound: handed back, never a wait");
        // The worker catches up: order kept, nothing lost.
        assert_eq!(rx.recv(), Ok(1));
        r.flush();
        assert_eq!(r.backlog(), 1);
        assert_eq!(rx.recv(), Ok(2));
        assert_eq!(r.hand(5), Ok(()), "3 moves first, 5 waits behind it");
        assert_eq!(rx.recv(), Ok(3));
        r.flush();
        assert_eq!(rx.recv(), Ok(5));
        assert_eq!(r.backlog(), 0);
        drop(rx);
        assert_eq!(r.hand(6), Err(6), "worker gone: handed back");
    }

    /// The spawned worker runs every value, in order.
    #[test]
    fn a_spawned_reaper_runs_every_value() {
        let (done_tx, done_rx) = std::sync::mpsc::channel::<u32>();
        let done_tx = std::sync::Mutex::new(done_tx);
        let r = Reaper::spawn("kf-test-reaper", 2, 2, move |v: u32| {
            let _ = done_tx.lock().map(|t| t.send(v));
        })
        .expect("thread");
        for v in 0..4 {
            assert_eq!(r.hand(v), Ok(()), "queue 2 + overflow 2 hold four");
        }
        let mut got = Vec::new();
        while got.len() < 4 {
            r.flush();
            if let Ok(v) = done_rx.recv_timeout(std::time::Duration::from_secs(5)) {
                got.push(v);
            } else {
                break;
            }
        }
        assert_eq!(got, vec![0, 1, 2, 3]);
    }
}
