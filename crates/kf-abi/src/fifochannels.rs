//! The `GSP_RM_CONTROL` reply body for `NV2080_CTRL_CMD_INTERNAL_FIFO_GET_NUM_CHANNELS`
//! (`0x20800a61`) — eight bytes, and the first control in the measured sweep whose refusal
//! **halts the boot** rather than amputating something.
//!
//! ## ★★★ Why this one is not an amputation
//!
//! `kfifoRunlistQueryNumChannels_KERNEL` issues it and, on any failure, returns **zero**
//! after a `DBG_BREAKPOINT()` that is defined empty on a release module
//! (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_fifo.c:1316-1341`;
//! `ogkm-580: src/nvidia/inc/kernel/core/printf.h:153`). The zero travels one frame up into
//! `kfifoChidMgrConstruct`:
//!
//! ```c
//! if (pChidMgr->numChannels == 0)
//! {
//!     if (kfifoChidMgrGetNumChannels(pGpu, pKernelFifo, pChidMgr) == 0)
//!     {
//!         NV_PRINTF(LEVEL_ERROR, "pChidMgr->numChannels is 0\n");
//!         DBG_BREAKPOINT();
//!         return NV_ERR_INVALID_STATE;
//!     }
//! }
//! ```
//!
//! (`ogkm-580: kernel_fifo.c:300-308`). ★ `NV_ERR_INVALID_STATE` is **not**
//! `NV_ERR_NOT_SUPPORTED`, so `gpuStateInit_IMPL`'s loop does not map it to `NV_OK` — it
//! takes the `goto gpuStateInit_exit` at `ogkm-580: gpu.c:2288-2289` and the boot aborts at
//! a named statement. ⊘ That is the *good* failure shape: loud, attributable, and one rung.
//! It is why `kayfabe_device::sweep` triages this control `RefusalHalts` and not as an
//! amputation.
//!
//! ⚠ It is also why serving it is what buys the next boot any reach at all. Every engine
//! after `KernelFifo` in `gpuChildOrderList_GM200` is unreachable while this returns zero.
//!
//! ## ★★ `runlistId` is `[IN]`, and this port echoes it
//!
//! The struct is `{ NvU32 runlistId; /* [IN] */ NvU32 numChannels; /* [OUT] */ }`
//! (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080internal.h:1573-1581`).
//! [`encode_fifo_num_channels`] takes the guest's own `runlistId` and writes it back
//! unchanged. ⊘ RM does not read the field again — `kfifoRunlistQueryNumChannels_KERNEL`
//! copies out `numChannelsParams.numChannels` and nothing else — so echoing is not a
//! mechanism, it is the refusal to overwrite a guest's own `[IN]` value with a different
//! number.
//!
//! ⚠ This is the one place this port's reply is **not** byte-identical to the oracle's for
//! every ask. The C splices one canned eight-byte answer with `runlistId = 0` for all four
//! asks (`C: src/qemu/mode2_initctrl_ga106.h:6210`, `ctl_20800a61[] = {0,0,0,0, 0x00,0x08,0,0}`
//! at `:12-14`, `nvidia-gpu-passthrough` rev `018e492`); we answer runlist 1 with
//! `runlistId = 1`.
//!
//! ## ★★ Where the count comes from, and what is assumed
//!
//! `numChannels = 0x800` = **2048**, read out of that same captured reply — a real RTX
//! 3060's own GSP answering this control. `[measured]`, and the run is the **C artifact's
//! rather than this port's**.
//!
//! ⊘ `[inferred]`: that *every* runlist has the same count. The oracle's table is keyed on
//! the command alone, so the C answered all four of `cap1b`'s asks (`rpc.sequence` 15, 16,
//! 17 and 34) with the same 2048, and a stock guest booted through it to
//! `cuCtxCreate → 2048² matmul` at `bad=0 maxerr=0`. That is the strongest evidence
//! available and it is not a measurement of per-runlist counts. A chip whose runlists
//! differ needs a slice here, not a scalar, and this row would be that change.
//!
//! ⚠ `[measured]` in `cap1b` the guest asks this control at `rpc.sequence` **15, 16, 17**
//! (txns 991-993) and **34** (txn 1010), `paylen 48` = 40 header + 8 params, all inside the
//! replay's closure limit of 1028.

/// `NV2080_CTRL_CMD_INTERNAL_FIFO_GET_NUM_CHANNELS`
/// (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080internal.h:1576`).
pub const NV2080_CTRL_CMD_INTERNAL_FIFO_GET_NUM_CHANNELS: u32 = 0x2080_0a61;

/// Byte offset of `runlistId` — `[IN]`, echoed.
pub const RUNLIST_ID_OFF: usize = 0;

/// Byte offset of `numChannels` — `[OUT]`.
pub const NUM_CHANNELS_OFF: usize = 4;

/// `sizeof(NV2080_CTRL_INTERNAL_FIFO_GET_NUM_CHANNELS_PARAMS)`.
pub const FIFO_NUM_CHANNELS_PARAMS_SIZE: usize = NUM_CHANNELS_OFF + 4;

/// ★★ **How many channel ids this chip's FIFO has per runlist.**
///
/// ⊘ A scalar, not a per-runlist slice, and the docs above say exactly what that assumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FifoChannelsRow {
    /// `numChannels` for every runlist.
    pub channels_per_runlist: u32,
}

/// Why the reply could not be encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FifoChannelsError {
    /// ★★★ **A channel count of zero, which is the failure this control exists to avoid.**
    ///
    /// `kfifoChidMgrConstruct` treats zero as `NV_ERR_INVALID_STATE` and aborts
    /// `gpuStateInit` (`ogkm-580: kernel_fifo.c:300-308`), and if it did not, the two
    /// `constructObjEHeap(…, 0, numChannels, …)` calls that follow (`:309-331`) would build
    /// channel-id heaps with no extent. ⊘ Encoding it would be answering `NV_OK` with the
    /// content of a refusal — strictly worse than refusing, because the envelope then says
    /// the answer is good.
    NoChannels,
}

impl core::fmt::Display for FifoChannelsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoChannels => write!(
                f,
                "channels_per_runlist is 0; kfifoChidMgrConstruct reads that as \
                 NV_ERR_INVALID_STATE and aborts gpuStateInit (kernel_fifo.c:300-308)"
            ),
        }
    }
}

impl core::error::Error for FifoChannelsError {}

/// Encode `NV2080_CTRL_INTERNAL_FIFO_GET_NUM_CHANNELS_PARAMS`, echoing the guest's own
/// `[IN]` `runlist_id`.
///
/// # Errors
///
/// [`FifoChannelsError::NoChannels`].
pub fn encode_fifo_num_channels(
    row: &FifoChannelsRow,
    runlist_id: u32,
) -> Result<Vec<u8>, FifoChannelsError> {
    if row.channels_per_runlist == 0 {
        return Err(FifoChannelsError::NoChannels);
    }
    let mut params = vec![0u8; FIFO_NUM_CHANNELS_PARAMS_SIZE];
    params[RUNLIST_ID_OFF..RUNLIST_ID_OFF + 4].copy_from_slice(&runlist_id.to_le_bytes());
    params[NUM_CHANNELS_OFF..NUM_CHANNELS_OFF + 4]
        .copy_from_slice(&row.channels_per_runlist.to_le_bytes());
    Ok(params)
}
