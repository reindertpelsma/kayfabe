//! The `GSP_RM_CONTROL` reply body for
//! `NV2080_CTRL_CMD_INTERNAL_CONF_COMPUTE_GET_STATIC_INFO` (`0x20800af3`) — two `NvBool`s,
//! and the only thing standing between CPU-RM and a BAR1 mapping of protected vidmem.
//!
//! ## ★★ Where the guest asks it, and why refusing does not stop anything
//!
//! Twice, from two different phases of the same engine:
//!
//! - `confComputeStateInitLocked_IMPL` (`ogkm-580:
//!   src/nvidia/src/kernel/gpu/conf_compute/conf_compute.c:548-566`), under
//!   `NV_ASSERT_OK_OR_RETURN`, and the control is the *whole* of that function;
//! - `confComputeStatePostLoad_IMPL` (`:441-456`), again under `NV_ASSERT_OK_OR_RETURN`.
//!
//! ⊘ **Neither is the `gpuStatePreInit` sweep, and that matters.** `gpuStateInit_IMPL`'s
//! loop maps `NV_ERR_NOT_SUPPORTED` to `NV_OK` and does **not** remove the engine
//! (`ogkm-580: src/nvidia/src/kernel/gpu/gpu.c:2286-2287`); `gpuStatePostLoad` does exactly
//! the same (`:3437-3439`). So refusing this control amputates nothing and stops nothing —
//! it leaves `ConfidentialCompute` alive with a `ccStaticInfo` nobody wrote.
//!
//! ## ★★★ The §6 shape, and this is the sharpest instance of it yet
//!
//! `pConfCompute->ccStaticInfo` is a **member of the NVOC object**, zeroed when the object
//! is constructed. It is not re-zeroed before the call and it does not need to be: a
//! refusal leaves it zero, and a served all-zero reply writes zero. ⇒ **refusing and
//! serving the truth are byte-identical to the guest.** The only observable difference is
//! the RPC envelope's `rpc_result`, which a real GA106's GSP sets to `NV_OK`.
//!
//! `[measured]`, and the run is the **C artifact's rather than this port's**: the oracle's
//! own reply table carries `{0x20800af3u, 0x0u, 2u, 0u, ctl_20800af3}`
//! (`C: src/qemu/mode2_initctrl_ga106.h:6216`) and `ctl_20800af3[]` is **empty** (`:39-40`)
//! — the capture trims trailing zeros, so a real RTX 3060's GSP answered this control
//! `NV_OK` with **both bits clear**. That is the row [`crate::confcompute`] encodes, and it
//! is a statement rather than a default because the row has no `Default`.
//!
//! ⚠ So this module cannot make the fail-open combination unencodable the way
//! [`crate::memsysconfig`] does — here the fail-open bytes *are* the right bytes. What it
//! makes unencodable is the **opposite** direction, which is the one that widens.
//!
//! ## ★★★ Why `true` is the value this port may not say
//!
//! The pair is read in exactly one place that decides anything
//! (`ogkm-580: src/nvidia/src/kernel/rmapi/mapping_cpu.c:215-235`):
//!
//! ```c
//! if (((pCC != NULL) && !pCC->ccStaticInfo.bIsBar1Trusted &&
//!     !pCC->ccStaticInfo.bIsPcieTrusted) || …)
//! {
//!     NV_PRINTF(LEVEL_ERROR, "BAR1 mapping to CPR vidmem not supported\n");
//!     NV_ASSERT(0);
//!     return NV_ERR_NOT_SUPPORTED;
//! }
//! ```
//!
//! Both bits clear is RM **refusing to map protected vidmem through BAR1**. Either bit set
//! removes that refusal. This port serves no compute-protected region and no GSP-DMA
//! bounce path, so a `true` here would delete a guest-side safety check and back it with
//! nothing. [`ConfComputeError::TrustedWithoutCprPlane`] is that made unencodable.
//!
//! ⊘ Stated as a property of *this port*, not of the ABI: a port that grows a CPR aperture
//! grows the right to say `true`, and the check moves to a chip-row predicate at that
//! point. It is not moved pre-emptively, because a predicate with one possible answer is
//! not a predicate.
//!
//! ⚠ `[measured]` in `cap1b` the guest asks this control at `rpc.sequence` **13** (txn 989,
//! `paylen 42` = 40 header + 2 params) and again at `rpc.sequence` **44** (txn 1020), both
//! inside the replay's closure limit of 1028 — so
//! `crates/kayfabe-crec/tests/cap1b_differential.rs` exercises this reply twice.

/// `NV2080_CTRL_CMD_INTERNAL_CONF_COMPUTE_GET_STATIC_INFO`
/// (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080internal.h:3648`).
pub const NV2080_CTRL_CMD_INTERNAL_CONF_COMPUTE_GET_STATIC_INFO: u32 = 0x2080_0af3;

/// Byte offset of `bIsBar1Trusted`.
pub const BAR1_TRUSTED_OFF: usize = 0;

/// Byte offset of `bIsPcieTrusted`.
pub const PCIE_TRUSTED_OFF: usize = 1;

/// `sizeof(NV2080_CTRL_INTERNAL_CONF_COMPUTE_GET_STATIC_INFO_PARAMS)` — two `NvBool`s and
/// no padding (`ogkm-580: ctrl2080internal.h:3652-3655`).
///
/// The guest declares this in the control header and
/// `kayfabe_device::inittables::InitTablePolicy` refuses a request that disagrees, so it is
/// a wire fact rather than a convenience.
pub const CONF_COMPUTE_STATIC_INFO_PARAMS_SIZE: usize = PCIE_TRUSTED_OFF + 1;

/// ★★ **What this device says about its confidential-compute plane.**
///
/// ⊘ No `Default`, deliberately. Both fields being `false` is the right answer for this
/// device *and* what a refusal would leave behind — see this module's docs — so the only
/// thing that distinguishes a served answer from an accident is that someone wrote the
/// values down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfComputeRow {
    /// `bIsBar1Trusted` — whether RM may map compute-protected vidmem through BAR1.
    pub bar1_trusted: bool,
    /// `bIsPcieTrusted` — the same permission, granted for PCIe as a whole.
    pub pcie_trusted: bool,
}

/// Why the reply could not be encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfComputeError {
    /// ★★★ **A trust claim with nothing behind it.**
    ///
    /// `mapping_cpu.c:227-235` refuses to map CPR vidmem through BAR1 while **both** bits
    /// are clear, and stops refusing as soon as either is set. This port serves no
    /// compute-protected region, so setting one deletes a guest-side check and backs it
    /// with nothing.
    TrustedWithoutCprPlane {
        /// The offending `bIsBar1Trusted`.
        bar1_trusted: bool,
        /// The offending `bIsPcieTrusted`.
        pcie_trusted: bool,
    },
}

impl core::fmt::Display for ConfComputeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TrustedWithoutCprPlane {
                bar1_trusted,
                pcie_trusted,
            } => write!(
                f,
                "bIsBar1Trusted={bar1_trusted} bIsPcieTrusted={pcie_trusted}: either bit \
                 removes RM's own refusal to map CPR vidmem through BAR1 \
                 (mapping_cpu.c:227-235), and this port serves no compute-protected region"
            ),
        }
    }
}

impl core::error::Error for ConfComputeError {}

/// Encode `NV2080_CTRL_INTERNAL_CONF_COMPUTE_GET_STATIC_INFO_PARAMS` from a chip's row.
///
/// # Errors
///
/// [`ConfComputeError::TrustedWithoutCprPlane`] — see it for the guest-side check it stands
/// in front of.
pub fn encode_conf_compute_static_info(row: &ConfComputeRow) -> Result<Vec<u8>, ConfComputeError> {
    if row.bar1_trusted || row.pcie_trusted {
        return Err(ConfComputeError::TrustedWithoutCprPlane {
            bar1_trusted: row.bar1_trusted,
            pcie_trusted: row.pcie_trusted,
        });
    }
    let mut params = vec![0u8; CONF_COMPUTE_STATIC_INFO_PARAMS_SIZE];
    params[BAR1_TRUSTED_OFF] = u8::from(row.bar1_trusted);
    params[PCIE_TRUSTED_OFF] = u8::from(row.pcie_trusted);
    Ok(params)
}
