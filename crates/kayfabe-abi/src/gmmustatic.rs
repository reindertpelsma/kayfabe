//! The `GSP_RM_CONTROL` reply body for `NV2080_CTRL_CMD_INTERNAL_GMMU_GET_STATIC_INFO`
//! (`0x20800a59`) — sixteen bytes, four fault-buffer sizes, and ★★★ **the one control in the
//! measured sweep whose refusal leaves the guest holding a pointer to freed memory.**
//!
//! ## ★★★ Refusing this is a use-after-free, not an amputation
//!
//! `_kgmmuInitStaticInfo` (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:139-166`) is
//! four statements:
//!
//! ```c
//! pKernelGmmu->pStaticInfo = portMemAllocNonPaged(sizeof(*pKernelGmmu->pStaticInfo));
//! NV_CHECK_OR_RETURN(LEVEL_ERROR, pKernelGmmu->pStaticInfo != NULL, NV_ERR_INSUFFICIENT_RESOURCES);
//! portMemSet(pKernelGmmu->pStaticInfo, 0, sizeof(*pKernelGmmu->pStaticInfo));
//! NV_CHECK_OK_OR_GOTO(status, LEVEL_ERROR,
//!     pRmApi->Control(pRmApi, …, NV2080_CTRL_CMD_INTERNAL_GMMU_GET_STATIC_INFO,
//!                     pKernelGmmu->pStaticInfo, sizeof(*pKernelGmmu->pStaticInfo)), fail);
//! fail:
//!     if (status != NV_OK) { portMemFree(pKernelGmmu->pStaticInfo); }
//!     return status;
//! ```
//!
//! ⚠ **`portMemFree` without a `NULL` assignment.** The field keeps the freed address, and
//! `kgmmuStateInitLocked_IMPL` returns the failure straight into `gpuStateInit_IMPL`'s
//! loop — which maps `NV_ERR_NOT_SUPPORTED` to `NV_OK` and carries on
//! (`ogkm-580: src/nvidia/src/kernel/gpu/gpu.c:2286-2287`). `KernelGmmu` survives with
//! `pStaticInfo` **dangling**.
//!
//! ★★ That is strictly worse than the amputation class. `gpuDestroyMissingEngine` at least
//! NULLs the engine pointer, so a `NULL` check downstream can catch it
//! (`kayfabe_abi::memsysconfig`'s `t134a`). A freed-but-non-`NULL` pointer passes every
//! check RM makes, and `kgmmuGetStaticInfo` hands it out to control handlers a guest can
//! reach: `mmu_fault_buffer_ctrl.c:84` and `:176` both read
//! `pStaticInfo->…FaultBufferSize` out of it.
//!
//! ⊘ `[inferred]`, and the boundary is stated because it matters: **no boot has been spent
//! at a revision that serves this.** The reading above is of `ogkm-580` source. What is
//! *measured* is only that the refusal happens — `0x20800a59` is in `cap1b`'s prefix at
//! `rpc.sequence` 26 (txn 1002) and this port answered it `NV_ERR_NOT_SUPPORTED` before this
//! module existed. Whether the dangling read is reached during bring-up, or only from a
//! later guest-issued control, is **not** established here.
//!
//! ## ★★ What serving it changes downstream, stated up front
//!
//! Refusing returns from `kgmmuStateInitLocked_IMPL` at its *first* statement. Serving lets
//! that function continue into `kgmmuFaultBufferInit_HAL`
//! (`ogkm-580: kern_gmmu.c:190-193`), which this port answers with its deliberate refusal of
//! `NV2080_CTRL_CMD_INTERNAL_GMMU_REGISTER_FAULT_BUFFER`
//! (`kayfabe_device::faultbuffer`, gated by `docs/design/resume_from_fault.md` §7 step 0).
//! ⊘ That refusal is `NV_ERR_NOT_SUPPORTED`, which `gpuStateInit_IMPL` maps to `NV_OK`, so
//! the reading is that the boot survives it — **`[inferred]`, and it is the single thing
//! about this control a boot would settle.**
//!
//! ## ★★ Where the four numbers come from
//!
//! `[measured]`, and the run is the **C artifact's rather than this port's**: the oracle's
//! captured reply table carries `{0x20800a59u, 0x0u, 16u, 16u, ctl_20800a59}`
//! (`C: src/qemu/mode2_initctrl_ga106.h:6260`, `nvidia-gpu-passthrough` rev `018e492`) —
//! `psize` 16, captured `dlen` 16, nothing trimmed — and `ctl_20800a59[]` at `:5418-5419`
//! is
//!
//! ```text
//! 00 10 03 00   replayableFaultBufferSize                  = 0x0003_1000 (200 704)
//! 00 00 00 00   replayableShadowFaultBufferMetadataSize    = 0
//! 20 0c 12 00   nonReplayableFaultBufferSize               = 0x0012_0c20 (1 182 752)
//! 00 00 00 00   nonReplayableShadowFaultBufferMetadataSize = 0
//! ```
//!
//! ★ Self-checking, which is why the field order is trusted and not only the values: both
//! sizes are exact multiples of `NVC369_BUF_SIZE` = 32, the fault packet size RM divides
//! them by (`ogkm-580: src/common/sdk/nvidia/inc/class/clc369.h:35`;
//! `kern_gmmu.c:1725`, `kern_gmmu_gv100.c:956, 1357`) — 6 272 and 36 961 packets. A
//! transposed field order would put a zero where a size belongs and the multiple-of-32
//! property would be vacuous rather than confirmatory.

/// `NV2080_CTRL_CMD_INTERNAL_GMMU_GET_STATIC_INFO`
/// (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080internal.h:1414`).
pub const NV2080_CTRL_CMD_INTERNAL_GMMU_GET_STATIC_INFO: u32 = 0x2080_0a59;

/// `NVC369_BUF_SIZE` — one `GMMU_FAULT_PACKET`
/// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc369.h:35`). RM divides both buffer sizes
/// by it to get a queue capacity.
pub const FAULT_PACKET_SIZE: u32 = 32;

/// Byte offset of `replayableFaultBufferSize`.
pub const REPLAYABLE_SIZE_OFF: usize = 0;

/// Byte offset of `replayableShadowFaultBufferMetadataSize`.
pub const REPLAYABLE_SHADOW_META_OFF: usize = 4;

/// Byte offset of `nonReplayableFaultBufferSize`.
pub const NON_REPLAYABLE_SIZE_OFF: usize = 8;

/// Byte offset of `nonReplayableShadowFaultBufferMetadataSize`.
pub const NON_REPLAYABLE_SHADOW_META_OFF: usize = 12;

/// `sizeof(NV2080_CTRL_INTERNAL_GMMU_GET_STATIC_INFO_PARAMS)` — four `NvU32`s
/// (`ogkm-580: ctrl2080internal.h:1417-1422`).
pub const GMMU_STATIC_INFO_PARAMS_SIZE: usize = NON_REPLAYABLE_SHADOW_META_OFF + 4;

/// Which of the two hardware fault buffers a size belongs to. Diagnostic only — carried in
/// [`GmmuStaticError`] so a refusal names the field rather than a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultBufferKind {
    /// `REPLAYABLE_FAULT_BUFFER`.
    Replayable,
    /// `NON_REPLAYABLE_FAULT_BUFFER`.
    NonReplayable,
}

impl core::fmt::Display for FaultBufferKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Replayable => write!(f, "replayable"),
            Self::NonReplayable => write!(f, "nonReplayable"),
        }
    }
}

/// ★★ **The two MMU fault buffers this chip's GSP sizes, and their shadow metadata.**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GmmuStaticRow {
    /// `replayableFaultBufferSize`, in bytes.
    pub replayable_size: u32,
    /// `replayableShadowFaultBufferMetadataSize`, in bytes.
    pub replayable_shadow_metadata_size: u32,
    /// `nonReplayableFaultBufferSize`, in bytes.
    pub non_replayable_size: u32,
    /// `nonReplayableShadowFaultBufferMetadataSize`, in bytes.
    pub non_replayable_shadow_metadata_size: u32,
}

/// Why the reply could not be encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GmmuStaticError {
    /// ★★★ **An invariant RM asserts on itself.**
    ///
    /// `kgmmuFaultBufferAllocSharedMemory_IMPL` opens with
    /// `NV_ASSERT_OR_RETURN(pStaticInfo->nonReplayableFaultBufferSize != 0,
    /// NV_ERR_INVALID_STATE)` (`ogkm-580: kern_gmmu.c:1909`). Same shape as
    /// [`crate::memsysconfig::ComptagAllocationPolicy`]'s missing *neither* variant: a value
    /// the guest would be handed and then reject.
    NonReplayableSizeZero,
    /// ★★ **A fault buffer with no entries.**
    ///
    /// `kgmmuFaultBufferReplayableAllocate_IMPL` allocates a buffer of exactly this many
    /// bytes (`ogkm-580: kern_gmmu.c:1218-1222`) and the queue capacity is
    /// `faultBufferSize / NVC369_BUF_SIZE` (`:1725`, `kern_gmmu_gv100.c:956, 1357`) — zero
    /// bytes is a fault queue that can hold no fault.
    ReplayableSizeZero,
    /// ★★ **A trailing partial fault packet.**
    ///
    /// Both sizes are divided by `NVC369_BUF_SIZE` = 32 to get a capacity, and RM
    /// `ct_assert`s the page/packet relationship on itself (`ogkm-580: kern_gmmu.c:1904`).
    /// A size that is not a multiple of the packet leaves a tail the capacity arithmetic
    /// discards and hardware may still address.
    SizeNotPacketAligned {
        /// Which buffer.
        which: FaultBufferKind,
        /// The offending size, in bytes.
        size: u32,
    },
}

impl core::fmt::Display for GmmuStaticError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NonReplayableSizeZero => write!(
                f,
                "nonReplayableFaultBufferSize is 0, which RM asserts against itself \
                 (NV_ASSERT_OR_RETURN, kern_gmmu.c:1909)"
            ),
            Self::ReplayableSizeZero => write!(
                f,
                "replayableFaultBufferSize is 0; the queue capacity is size / \
                 NVC369_BUF_SIZE (kern_gmmu.c:1725), so the buffer would hold no fault"
            ),
            Self::SizeNotPacketAligned { which, size } => write!(
                f,
                "{which}FaultBufferSize {size:#x} is not a multiple of NVC369_BUF_SIZE \
                 ({FAULT_PACKET_SIZE}); RM divides it to get a queue capacity \
                 (kern_gmmu.c:1725) and the remainder is a partial packet"
            ),
        }
    }
}

impl core::error::Error for GmmuStaticError {}

/// Encode `NV2080_CTRL_INTERNAL_GMMU_GET_STATIC_INFO_PARAMS` from a chip's row.
///
/// # Errors
///
/// Every variant of [`GmmuStaticError`]; see each for the guest-side consequence it stands
/// in front of.
pub fn encode_gmmu_static_info(row: &GmmuStaticRow) -> Result<Vec<u8>, GmmuStaticError> {
    if row.non_replayable_size == 0 {
        return Err(GmmuStaticError::NonReplayableSizeZero);
    }
    if row.replayable_size == 0 {
        return Err(GmmuStaticError::ReplayableSizeZero);
    }
    for (which, size) in [
        (FaultBufferKind::Replayable, row.replayable_size),
        (FaultBufferKind::NonReplayable, row.non_replayable_size),
    ] {
        if size % FAULT_PACKET_SIZE != 0 {
            return Err(GmmuStaticError::SizeNotPacketAligned { which, size });
        }
    }

    let mut params = vec![0u8; GMMU_STATIC_INFO_PARAMS_SIZE];
    let put = |p: &mut [u8], off: usize, v: u32| p[off..off + 4].copy_from_slice(&v.to_le_bytes());
    put(&mut params, REPLAYABLE_SIZE_OFF, row.replayable_size);
    put(
        &mut params,
        REPLAYABLE_SHADOW_META_OFF,
        row.replayable_shadow_metadata_size,
    );
    put(
        &mut params,
        NON_REPLAYABLE_SIZE_OFF,
        row.non_replayable_size,
    );
    put(
        &mut params,
        NON_REPLAYABLE_SHADOW_META_OFF,
        row.non_replayable_shadow_metadata_size,
    );
    Ok(params)
}
