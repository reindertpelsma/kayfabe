//! The `GSP_RM_CONTROL` reply body for
//! `NV2080_CTRL_CMD_INTERNAL_MEMSYS_GET_STATIC_CONFIG` — the control the guest's
//! `KernelMemorySystem` engine asks in its `StatePreInit`, and the **only** thing that
//! engine's `StatePreInit` does.
//!
//! ## ★★★ Why this one is not a rung like the five before it
//!
//! The five controls served before this were all reached from `gpuPreInit`, a chain of
//! `NV_ASSERT_OK_OR_GOTO`s: refusing one ended the function at a named statement and the
//! boot stopped, cleanly, at the caller that wanted it. This one is reached from
//! `gpuStatePreInit_IMPL`'s **engine sweep**, and refusing it does not stop anything. It
//! deletes an engine.
//!
//! `kmemsysStatePreInitLocked_IMPL` allocates the config, zeroes it, assigns the pointer,
//! and issues exactly one control under `NV_ASSERT_OK_OR_GOTO`
//! (`ogkm-580: src/nvidia/src/kernel/gpu/mem_sys/kern_mem_sys.c:99-134`, the control at
//! `:121-124`, forwarded by `kmemsysInitStaticConfig_KERNEL` at `:466-478`). There is
//! nothing else in it, so a refusal of `0x20800a1c` **is** the engine's `StatePreInit`
//! result, one for one.
//!
//! And `NV_ERR_NOT_SUPPORTED` is not an error to the sweep — it is an *engine-removal
//! request* (`ogkm-580: src/nvidia/src/kernel/gpu/gpu.c:2170-2214`):
//!
//! ```c
//! if (rmStatus == NV_ERR_NOT_SUPPORTED)
//! {
//!     switch (curEngDescriptor) { /* three whitelisted engines … */
//!         default:
//!             NV_PRINTF(LEVEL_ERROR,
//!                 "disallowing NV_ERR_NOT_SUPPORTED PreInit removal of untracked engine (%s)\n",
//!                 engstateGetName(pEngstate));
//!             DBG_BREAKPOINT();
//!             NV_ASSERT(0);
//!             break;
//!     }
//!     gpuDestroyMissingEngine(pGpu, pEngstate);
//!     pEngstate = NULL;
//!     rmStatus = gpuDeleteEngineOnPreInit(pGpu, curEngDescriptor);
//! }
//! else if (rmStatus != NV_OK) { break; }
//! ```
//!
//! ⚠ **The word *disallowing* is a lie, and it cost a boot to find out.** The `default:`
//! arm only logs and asserts; it then `break`s out of the *switch* and falls into
//! `gpuDestroyMissingEngine` at `:2208` anyway. On a release module neither guard has any
//! control-flow effect — `DBG_BREAKPOINT()` is defined empty
//! (`ogkm-580: src/nvidia/inc/kernel/core/printf.h:153`) and `NV_ASSERT(0)` expands to a
//! logging macro with *"no other action"* (`ogkm-580:
//! src/nvidia/inc/libraries/utils/nvassert.h:336`). `KernelMemorySystem` is not one of the
//! three whitelisted descriptors, so it is error-printed and then removed identically.
//!
//! ★★ And `:2211` **overwrites `rmStatus`** with `gpuDeleteEngineOnPreInit`'s `NV_OK`, so
//! `gpuStatePreInit_IMPL` returns `NV_OK` (`:2229`) after amputating the engine and
//! `gpumgrStatePreInitGpu` records success (`ogkm-580:
//! src/nvidia/src/kernel/gpu_mgr/gpu_mgr.c:2024`). The boot sails on into `gpuStateInit`.
//!
//! ## ★★★ What that cost, `[measured]`
//!
//! `[measured]` run `t134a`, a stock 580.159.04 guest at `1c79474` driven with
//! `nvidia-smi`. `GPU_GET_KERNEL_MEMORY_SYSTEM(pGpu)` is a generated accessor that reads
//! `pGpu->children.named.pKernelMemorySystem` (`ogkm-580:
//! src/nvidia/inc/kernel/gpu/gpu_child_list.h:85`, expanded at
//! `src/nvidia/generated/g_gpu_nvoc.h:5579-5581`), and `gpuDestroyMissingEngine` NULLs that
//! slot (`gpu.c:1877-1882`). Several thousand lines later:
//!
//! ```text
//! NVRM: kmemsysInitStaticConfig_HAL(…) @ kern_mem_sys.c:122
//! NVRM: gpuStatePreInit_IMPL: disallowing NV_ERR_NOT_SUPPORTED PreInit removal of
//!       untracked engine (KernelMemorySystem:0)
//! NVRM: nvAssertFailedNoLog: Assertion failed: pKernelMemorySystem != NULL @ kern_mem_sys.c:364
//! BUG: kernel NULL pointer dereference, address: 0000000000000268
//! RIP: 0010:memmgrGetBlackListPagesForHeap_GM107+0x23/0x140 [nvidia]
//!   heapInit_IMPL ← memmgrCreateHeap_IMPL ← memmgrStateInitLocked_IMPL ← gpuStateInit_IMPL
//!   ← gpumgrStateInitGpu ← RmInitAdapter ← nv_open_device
//! ```
//!
//! ★ The faulting instruction is worth reading, because it says *why* no amount of
//! defensiveness inside RM would have helped. The disassembly in the Oops is
//! `mov rsi,[rdi+0x1948]` — `GPU_GET_KERNEL_MEMORY_SYSTEM`, which loaded `0` — followed by
//! `mov rax,[rsi+0x268]`, and `CR2` is `0x268`. That second load is
//! `kmemsysGetMaximumBlacklistPages`, which is **not a function**: it is an NVOC virtual
//! dispatch that reads the vtable slot *out of the object*
//! (`ogkm-580: src/nvidia/generated/g_kern_mem_sys_nvoc.h:929-931`,
//! `return pKernelMemorySystem->__kmemsysGetMaximumBlacklistPages__(…)`). The fault happens
//! before any callee body runs.
//!
//! ⚠ `nvidia-smi` therefore **hangs** instead of failing, because the Oops kills the
//! `nv_open_q` kthread mid-`RmInitAdapter`. That is a different observable from every
//! earlier rung and must not be read as progress stalling — it is the guest kernel damaged,
//! not the guest kernel refusing.
//!
//! ## ⊘ The two escape hatches that are not escape hatches
//!
//! - **Refuse with a different status.** `gpu.c:2215-2218`'s `else if (rmStatus != NV_OK)
//!   break;` would abort the sweep and propagate, so there is no Oops — but
//!   `gpuStatePreInit` then returns the error and `RmInitAdapter` fails. It trades a crash
//!   for a clean early abort. It never boots, and it would also destroy the one property
//!   that makes the sweep cheap to work with: that a single boot enumerates *every* control
//!   the whole engine list wants (`crate::…`, see `kayfabe_device::unserviced`).
//! - **Answer `NV_OK` with a zeroed struct.** The engine survives, and that is exactly the
//!   trap — see the next section.
//!
//! ## ★★★ Inert is WRONG here, and the bytes are identical either way
//!
//! `kmemsysStatePreInitLocked_IMPL:114` `portMemSet`s the config to zero **before** the
//! call. So an [`kayfabe_device::inert`]-style empty reply and a served all-zero reply
//! reach RM as the same forty bytes. They are not the same statement, and here they are not
//! even both survivable:
//!
//! ★★ **All-zero violates an invariant RM asserts on itself.**
//! `kmemsysAllocComprResources_KERNEL` opens with
//!
//! ```c
//! NV_ASSERT_OR_RETURN(pMemorySystemConfig->bOneToOneComptagLineAllocation ||
//!                     pMemorySystemConfig->bUseRawModeComptaglineAllocation,
//!     NV_ERR_INVALID_STATE);
//! ```
//!
//! (`ogkm-580: kern_mem_sys.c:422-423`, and the same disjunction is `NV_ASSERT`ed again at
//! `src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:951`). A reply with neither bit set makes
//! every compressed allocation fail `NV_ERR_INVALID_STATE`, from a config the guest was
//! handed and believed. [`ComptagAllocationPolicy`] is that disjunction made structural:
//! the row cannot express *neither*.
//!
//! ★★ **`comprPageSize == 0` is an integer divide-by-zero in the guest kernel.**
//!
//! ```c
//! NvU64 comprPageSize = pMemorySystemConfig->comprPageSize;
//! *pMemSize = ((*pMemSize + alignPad + comprPageSize - 1) / comprPageSize) * comprPageSize;
//! ```
//!
//! (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/arch/maxwell/mem_mgr_gm107.c:210-211`).
//! Four lines later the same field is used as an alignment **mask**,
//! `hwAlignment = NV_MAX(hwAlignment, comprPageSize - 1)` (`:216`, and
//! `mem_mgr_ga100.c:93`), which is only a mask at all if the value is a power of two —
//! hence [`MemorySystemError::ComprPageSizeNotPowerOfTwo`] as well as the zero check.
//!
//! ★★ **`ltcCount == 0` or `ltsPerLtcCount == 0` breaks the Ampere memory-boundary math.**
//! `kern_mem_sys_ga100.c:332-334` multiplies both into a shifted address and `:342-345`
//! branches on their product; `kern_mem_sys_ga102.c:66-120` matches the product against
//! `48`, `40`, `4×8` and `3×8`.
//!
//! ⊘ **What I did *not* find, stated so nobody re-derives it.** `l2CacheSize` reaches
//! `kbusInitBar0Window` at `ogkm-580: kern_bus_gm107.c:233-244`, where it looked like an
//! unsigned underflow — `offsetBar0 = l2CacheSize - DRF_SIZE(NV_PRAMIN)`. It is **guarded**:
//! `if (pMemorySystemConfig->l2CacheSize < DRF_SIZE(NV_PRAMIN)) offsetBar0 = 0;` on the line
//! above. There is no underflow and this module does not claim one.
//!
//! ★ What that reading *did* turn up is real, and is [`MemorySystemError::FbpaAbsent`]: the
//! whole block is entered only `if (gpuIsCacheOnlyModeEnabled(pGpu) ||
//! !(pMemorySystemConfig->bFbpaPresent))` (`:230-231`). Answering `bFbpaPresent = false`
//! therefore sends RM down a *different* BAR0-window placement, derived from `l2CacheSize`,
//! than the fixed `PRAMIN` window this device's register plane actually decodes
//! (`kayfabe_device::ChipProfile::pramin_window`). This port serves one placement, so it
//! may only advertise the field value that selects it.
//!
//! ## ★★ What the oracle settled
//!
//! Every value in [`crate::memsysconfig`]'s GA106 row is a real RTX 3060's own answer.
//! `C: src/qemu/mode2_initctrl_ga106.h:5391` (`ctl_20800a1c`, registered at `:6255` as
//! `{0x20800a1cu, 0x0u, 40u, 40u, ctl_20800a1c}` — `psize` 40, captured `dlen` 40, nothing
//! trimmed) is forty bytes captured from the host's own GSP
//! (`C: docs/research/captures/ga106_initctrl_580.log`):
//!
//! ```text
//! 00 00 01 00 00 00 00 00   bUseRawModeComptaglineAllocation = 1, the other six false
//! 00 00 24 00 00 00 00 00   l2CacheSize      = 0x24_0000 (2.25 MiB)
//! 01 00 00 00 00 00 01 00   bFbpaPresent = 1;  comprPageSize  = 0x1_0000 (64 KiB)
//! 10 00 00 00 11 00 00 00   comprPageShift   = 16;  ramType    = 0x11
//! 06 00 00 00 04 00 00 00   ltcCount         = 6;   ltsPerLtc  = 4
//! ```
//!
//! ★★ It is **self-checking**, which is why I trust the layout and not just the values:
//! `comprPageShift` is `log2(comprPageSize)` (16 = log2(65536)) exactly as the header's
//! `/*! log32(comprPageSize) */` comment intends despite its typo; `ramType = 0x11` is
//! `NV2080_CTRL_FB_INFO_RAM_TYPE_GDDR6` (`ogkm-580: ctrl2080fb.h:394`), which is what an
//! RTX 3060 has; and `ltcCount × ltsPerLtcCount = 24` slices at 96 KiB each is the
//! `0x24_0000` L2 in the field two rows above it. Four independent facts that agree.
//!
//! ⊘ And the arithmetic pins the byte offsets: `NV_DECLARE_ALIGNED(NvU64 l2CacheSize, 8)`
//! puts a pad byte at 7 and `bFbpaPresent` at 16 leaves three pad bytes before
//! `comprPageSize` at 20. A FINN-unaware layout would put `l2CacheSize` at 7 and every
//! later field would be wrong; the capture is the witness that it does not.
//!
//! ⚠ `[measured]` in `cap1b` the guest asks this control at `rpc.sequence` **11**, txn 987,
//! `paylen 80` = 40 (`RpcControlReq::HEADER`) + 40 (params), one queue element — so it is
//! inside the replay's closure limit of 1028 and
//! `crates/kayfabe-crec/tests/cap1b_differential.rs` exercises this reply.

/// `NV2080_CTRL_CMD_INTERNAL_MEMSYS_GET_STATIC_CONFIG`
/// (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080internal.h:143`).
pub const NV2080_CTRL_CMD_INTERNAL_MEMSYS_GET_STATIC_CONFIG: u32 = 0x2080_0a1c;

/// Byte offset of `bOneToOneComptagLineAllocation`.
pub const ONE_TO_ONE_COMPTAG_OFF: usize = 0;

/// Byte offset of `bUseOneToFourComptagLineAllocation`.
pub const ONE_TO_FOUR_COMPTAG_OFF: usize = 1;

/// Byte offset of `bUseRawModeComptaglineAllocation`.
pub const RAW_MODE_COMPTAG_OFF: usize = 2;

/// Byte offset of `bDisableCompbitBacking`.
pub const DISABLE_COMPBIT_BACKING_OFF: usize = 3;

/// Byte offset of `bDisablePostL2Compression`.
pub const DISABLE_POST_L2_COMPRESSION_OFF: usize = 4;

/// Byte offset of `bEnabledEccFBPA`.
pub const ENABLED_ECC_FBPA_OFF: usize = 5;

/// Byte offset of `bL2PreFill`.
pub const L2_PREFILL_OFF: usize = 6;

/// Byte offset of `l2CacheSize`.
///
/// ★ 8, not 7. The field is `NV_DECLARE_ALIGNED(NvU64 l2CacheSize, 8)` after seven
/// `NvBool`s, so one pad byte is inserted at offset 7. The oracle's capture is the witness
/// — see this module's docs.
pub const L2_CACHE_SIZE_OFF: usize = 8;

/// Byte offset of `bFbpaPresent`.
pub const FBPA_PRESENT_OFF: usize = 16;

/// Byte offset of `comprPageSize`.
///
/// ★ 20, not 17: an `NvU32` follows a lone `NvBool`, so three pad bytes are inserted.
pub const COMPR_PAGE_SIZE_OFF: usize = 20;

/// Byte offset of `comprPageShift`.
pub const COMPR_PAGE_SHIFT_OFF: usize = 24;

/// Byte offset of `ramType`.
pub const RAM_TYPE_OFF: usize = 28;

/// Byte offset of `ltcCount`.
pub const LTC_COUNT_OFF: usize = 32;

/// Byte offset of `ltsPerLtcCount`.
pub const LTS_PER_LTC_COUNT_OFF: usize = 36;

/// `sizeof(NV2080_CTRL_INTERNAL_MEMSYS_GET_STATIC_CONFIG_PARAMS)`.
///
/// The guest declares this in the control header and
/// `kayfabe_device::inittables::InitTablePolicy` refuses a request that disagrees, so it is
/// a wire fact rather than a convenience.
pub const MEMSYS_STATIC_CONFIG_PARAMS_SIZE: usize = LTS_PER_LTC_COUNT_OFF + 4;

/// `NV2080_CTRL_FB_INFO_RAM_TYPE_GDDR6` (`ogkm-580: ctrl2080fb.h:394`).
pub const RAM_TYPE_GDDR6: u32 = 0x11;

/// ★★★ **Which comptagline allocation policy this device claims — and why *neither* is
/// not one of the choices.**
///
/// RM asserts the disjunction on itself, twice:
///
/// ```c
/// NV_ASSERT_OR_RETURN(pMemorySystemConfig->bOneToOneComptagLineAllocation ||
///                     pMemorySystemConfig->bUseRawModeComptaglineAllocation,
///     NV_ERR_INVALID_STATE);
/// ```
///
/// (`ogkm-580: kern_mem_sys.c:422-423`; again as a bare `NV_ASSERT` at
/// `ogkm-580: kern_gmmu.c:951`). An enum with no *neither* is that assert made
/// unencodable — the zero-filled reply the guest would otherwise have believed cannot be
/// built here at all.
///
/// ⊘ **`bUseOneToFourComptagLineAllocation` is deliberately not a variant.** The field
/// exists in the ABI, but RM's disjunction above does **not** accept it: a config whose
/// only policy bit is the one-to-four bit fails `kmemsysAllocComprResources_KERNEL`'s
/// assert exactly as an all-zero config does. Offering it as a third variant would be
/// offering a policy that is expressible on the wire and rejected by the guest — which is
/// the shape of defect this crate exists to prevent. If a chip ever needs it, it needs it
/// *alongside* one of these two, and that is a different type than this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComptagAllocationPolicy {
    /// `bOneToOneComptagLineAllocation` — RM allocates one comptagline per compressible
    /// page and tracks them itself.
    OneToOne,
    /// `bUseRawModeComptaglineAllocation` — hardware owns the comptaglines and RM pins
    /// `compTagLineMin = 1` (`ogkm-580: kern_gmmu.c:955-959`).
    ///
    /// ★ This is what a real GA106's GSP reports, and it is the one this port can back:
    /// raw mode is precisely the mode in which RM does *not* expect a comptag allocator
    /// behind the reply.
    Raw,
}

/// ★★ **What this chip says about its memory system — the half that is static for the
/// whole life of the GPU.**
///
/// ⊘ `comprPageShift` is **not a field here.** It is `log2(comprPageSize)`, and a row that
/// carried both could state them inconsistently; [`encode_memsys_static_config`] derives
/// it. Same discipline as [`crate::falconinfo::FalconInventoryRow`] having no count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemorySystemRow {
    /// Which comptagline policy to advertise. See [`ComptagAllocationPolicy`].
    pub comptag_policy: ComptagAllocationPolicy,
    /// `bDisableCompbitBacking` — read by `memmgrStatePreInitLocked_IMPL`
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/mem_mgr.c:398-399`).
    pub disable_compbit_backing: bool,
    /// `bDisablePostL2Compression`.
    pub disable_post_l2_compression: bool,
    /// `bEnabledEccFBPA` — read by the heap's blacklist walk
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/heap.c:3920,3996`).
    pub ecc_fbpa_enabled: bool,
    /// `bL2PreFill`.
    pub l2_prefill: bool,
    /// `l2CacheSize`, in bytes.
    pub l2_cache_size: u64,
    /// `bFbpaPresent` — see [`MemorySystemError::FbpaAbsent`] for why this port may only
    /// say `true`.
    pub fbpa_present: bool,
    /// `comprPageSize` — the extent one comptag covers, in bytes. Must be a non-zero power
    /// of two; `comprPageShift` is derived from it.
    pub compr_page_size: u32,
    /// `ramType`, an `NV2080_CTRL_FB_INFO_RAM_TYPE_*`.
    pub ram_type: u32,
    /// `ltcCount` — level-2 cache slices' controllers.
    pub ltc_count: u32,
    /// `ltsPerLtcCount` — slices per controller.
    pub lts_per_ltc_count: u32,
}

/// Why the reply could not be encoded.
///
/// Every variant stands in front of a *specific* guest-side consequence, and each one is
/// reachable from an all-zero row — which is the point, because an all-zero row is what
/// this device would otherwise answer by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemorySystemError {
    /// ★★★ **A guest-kernel integer divide-by-zero.**
    ///
    /// `mem_mgr_gm107.c:210-211` divides the requested allocation size by this field with
    /// no guard: `*pMemSize = ((*pMemSize + alignPad + comprPageSize - 1) / comprPageSize)
    /// * comprPageSize`.
    ComprPageSizeZero,
    /// ★★ **An alignment mask that is not a mask.**
    ///
    /// The same field is used as `comprPageSize - 1` to build a hardware alignment mask
    /// (`ogkm-580: mem_mgr_gm107.c:216`, `mem_mgr_ga100.c:93`), which only masks anything if
    /// the value is a power of two. A non-power-of-two also has no exact
    /// `comprPageShift`, so the two fields could not be made to agree.
    ComprPageSizeNotPowerOfTwo {
        /// The offending `comprPageSize`.
        compr_page_size: u32,
    },
    /// ★★ **Memory-boundary arithmetic with a zero factor.**
    ///
    /// `kern_mem_sys_ga100.c:332-334` forms `… * ltcCount * ltsPerLtcCount >> 4` and
    /// `:342-345` branches on the product; `kern_mem_sys_ga102.c:66-120` matches it against
    /// `48`, `40`, `4×8` and `3×8`. A zero factor makes every one of those arms wrong.
    NoLtcSlices {
        /// The offending `ltcCount`.
        ltc_count: u32,
        /// The offending `ltsPerLtcCount`.
        lts_per_ltc_count: u32,
    },
    /// ★★ **A framebuffer-partition claim this device's register plane cannot back.**
    ///
    /// `kbusInitBar0Window` re-places the BAR0 window from `l2CacheSize` when
    /// `!bFbpaPresent` (`ogkm-580: kern_bus_gm107.c:230-247`). This port decodes one fixed
    /// `PRAMIN` window — `kayfabe_device::ChipProfile::pramin_window` — so it may only
    /// advertise the value that selects that placement.
    FbpaAbsent,
    /// ★ **A zero L2.**
    ///
    /// Not a crash anywhere I could find — `kern_bus_gm107.c:233` guards its subtraction —
    /// but it is read as a size by `memmgrCalcReservedFbSpace`
    /// (`ogkm-580: mem_mgr_gm107.c:974`), and a cache of zero bytes is not a true statement
    /// about any GPU this port emulates.
    L2CacheSizeZero,
}

impl core::fmt::Display for MemorySystemError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ComprPageSizeZero => write!(
                f,
                "comprPageSize is 0; RM divides an allocation size by it unguarded \
                 (mem_mgr_gm107.c:211), which is a guest-kernel divide-by-zero"
            ),
            Self::ComprPageSizeNotPowerOfTwo { compr_page_size } => write!(
                f,
                "comprPageSize {compr_page_size:#x} is not a power of two; RM uses \
                 comprPageSize-1 as an alignment mask (mem_mgr_gm107.c:216) and \
                 comprPageShift has no exact value"
            ),
            Self::NoLtcSlices {
                ltc_count,
                lts_per_ltc_count,
            } => write!(
                f,
                "ltcCount {ltc_count} x ltsPerLtcCount {lts_per_ltc_count} has a zero \
                 factor; the Ampere memory-boundary math multiplies and branches on the \
                 product (kern_mem_sys_ga100.c:332-345)"
            ),
            Self::FbpaAbsent => write!(
                f,
                "bFbpaPresent is false, which sends RM down the l2CacheSize-derived BAR0 \
                 window placement (kern_bus_gm107.c:230-247); this device serves one fixed \
                 PRAMIN window"
            ),
            Self::L2CacheSizeZero => write!(f, "l2CacheSize is 0"),
        }
    }
}

impl core::error::Error for MemorySystemError {}

/// Encode `NV2080_CTRL_INTERNAL_MEMSYS_GET_STATIC_CONFIG_PARAMS` from a chip's row.
///
/// The whole [`MEMSYS_STATIC_CONFIG_PARAMS_SIZE`]-byte struct is returned, with the two
/// pad runs the FINN layout inserts left zero.
///
/// ★ `comprPageShift` is written as `compr_page_size.trailing_zeros()` — derived, never
/// carried — so the two fields cannot disagree. See [`MemorySystemRow`].
///
/// # Errors
///
/// Every variant of [`MemorySystemError`]; see each for the guest-side consequence it
/// stands in front of.
pub fn encode_memsys_static_config(row: &MemorySystemRow) -> Result<Vec<u8>, MemorySystemError> {
    if row.compr_page_size == 0 {
        return Err(MemorySystemError::ComprPageSizeZero);
    }
    if !row.compr_page_size.is_power_of_two() {
        return Err(MemorySystemError::ComprPageSizeNotPowerOfTwo {
            compr_page_size: row.compr_page_size,
        });
    }
    if row.ltc_count == 0 || row.lts_per_ltc_count == 0 {
        return Err(MemorySystemError::NoLtcSlices {
            ltc_count: row.ltc_count,
            lts_per_ltc_count: row.lts_per_ltc_count,
        });
    }
    if !row.fbpa_present {
        return Err(MemorySystemError::FbpaAbsent);
    }
    if row.l2_cache_size == 0 {
        return Err(MemorySystemError::L2CacheSizeZero);
    }

    let mut params = vec![0u8; MEMSYS_STATIC_CONFIG_PARAMS_SIZE];
    let put_bool = |p: &mut [u8], off: usize, v: bool| p[off] = u8::from(v);
    let put_u32 = |p: &mut [u8], off: usize, v: u32| {
        p[off..off + 4].copy_from_slice(&v.to_le_bytes());
    };

    // ⊘ Exactly one policy bit, and the one-to-four bit is never set: see
    // [`ComptagAllocationPolicy`] for why RM's own assert makes it unusable alone.
    put_bool(
        &mut params,
        ONE_TO_ONE_COMPTAG_OFF,
        matches!(row.comptag_policy, ComptagAllocationPolicy::OneToOne),
    );
    put_bool(
        &mut params,
        RAW_MODE_COMPTAG_OFF,
        matches!(row.comptag_policy, ComptagAllocationPolicy::Raw),
    );
    put_bool(
        &mut params,
        DISABLE_COMPBIT_BACKING_OFF,
        row.disable_compbit_backing,
    );
    put_bool(
        &mut params,
        DISABLE_POST_L2_COMPRESSION_OFF,
        row.disable_post_l2_compression,
    );
    put_bool(&mut params, ENABLED_ECC_FBPA_OFF, row.ecc_fbpa_enabled);
    put_bool(&mut params, L2_PREFILL_OFF, row.l2_prefill);
    params[L2_CACHE_SIZE_OFF..L2_CACHE_SIZE_OFF + 8]
        .copy_from_slice(&row.l2_cache_size.to_le_bytes());
    put_bool(&mut params, FBPA_PRESENT_OFF, row.fbpa_present);
    put_u32(&mut params, COMPR_PAGE_SIZE_OFF, row.compr_page_size);
    // ★ Derived from the size, never carried alongside it.
    put_u32(
        &mut params,
        COMPR_PAGE_SHIFT_OFF,
        row.compr_page_size.trailing_zeros(),
    );
    put_u32(&mut params, RAM_TYPE_OFF, row.ram_type);
    put_u32(&mut params, LTC_COUNT_OFF, row.ltc_count);
    put_u32(&mut params, LTS_PER_LTC_COUNT_OFF, row.lts_per_ltc_count);
    Ok(params)
}
