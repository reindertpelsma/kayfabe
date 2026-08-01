//! The `GSP_RM_CONTROL` reply body for `NV2080_CTRL_CMD_INTERNAL_BIF_GET_STATIC_INFO`
//! (`0x20800aac`) — four `NvBool`s that become four `KernelBif` PDB properties, two of which
//! point RM at hardware this port does not have.
//!
//! ## ★★ Where the guest asks it, and the status nobody reads
//!
//! `kbifStateInitLocked_IMPL` calls `kbifStaticInfoInit(pGpu, pKernelBif);`
//! (`ogkm-580: src/nvidia/src/kernel/gpu/bif/kernel_bif.c:132`) — **and discards the return
//! value**. Every other call in that function is wrapped in `NV_CHECK_OK_OR_RETURN`; this
//! one is a bare statement.
//!
//! ⇒ Refusing this control is invisible twice over. `kbifStaticInfoInit` takes its
//! `NV_CHECK_OK_OR_GOTO` exit (`:412-417`), skipping the four `setProperty` calls at
//! `:418-425`; `kbifStateInitLocked` returns `NV_OK` regardless; and `gpuStateInit_IMPL`
//! would have mapped a refusal to `NV_OK` anyway (`ogkm-580: gpu.c:2286-2287`). Nothing
//! logs above `LEVEL_ERROR`'s own line and no engine is removed.
//!
//! ## ★★★ The §6 shape, and here it is the *allocation* that fails open
//!
//! `kbifStaticInfoInit` `portMemAllocNonPaged`s the params and `portMemSet`s them to zero
//! before the call (`:401-409`). The four properties are `NV_FALSE` at construction. So a
//! refusal and a served all-false reply leave `KernelBif` in **the same state**, and the
//! only observable difference is the RPC envelope's `rpc_result`.
//!
//! `[measured]`, and the run is the **C artifact's rather than this port's**: the oracle's
//! reply table carries `{0x20800aacu, 0x0u, 4u, 0u, ctl_20800aac}`
//! (`C: src/qemu/mode2_initctrl_ga106.h:6258`) with an **empty** `ctl_20800aac[]`
//! (`:5413-5414`) — the capture trims trailing zeros, so a real RTX 3060's GSP answered
//! this control `NV_OK` with all four bits clear, `bPcieGen4Capable` included.
//!
//! ⚠ `bPcieGen4Capable = false` on a Gen4-capable board is worth reading twice before
//! copying it. It is what the oracle's own silicon reported and it is carried verbatim,
//! because this port has no independent source for it and no code path that reads the
//! property: `PDB_PROP_KBIF_PCIE_GEN4_CAPABLE` is *set* at `kernel_bif.c:418-419` and, in
//! the whole 580 open tree, read nowhere. ⊘ `[inferred]`, from a grep of the open trees
//! only — a closed userspace client is not greppable.
//!
//! ## ★★★ The two bits this port may not set, and what each points at
//!
//! - **`bIsC2CLinkUp`.** `kmemsysStateInitLocked_IMPL` branches on it twice
//!   (`ogkm-580: src/nvidia/src/kernel/gpu/mem_sys/kern_mem_sys.c:168, 342`) to set up a
//!   *coherent C2C* CPU mapping of framebuffer, and `kbifGetC2CLinkStatus`-adjacent Hopper
//!   code pairs two GPUs on it (`kernel_bif_gh100.c:1826-1827`). A GA10x consumer part has
//!   no C2C link at all; claiming one sends the guest's memory system down a mapping path
//!   this device's register plane never decodes.
//! - **`bIsDeviceMultiFunction`.** `_kbifSavePcieConfigRegisters` returns early unless it is
//!   set, and otherwise saves and restores configuration space **for function 1** through
//!   `pKernelBif->xveRegmapRef[1]` (`ogkm-580:
//!   src/nvidia/src/kernel/gpu/bif/arch/maxwell/kernel_bif_gm107.c:430-441`, again at
//!   `:668`). This port publishes a single-function device, so a multifunction claim points
//!   RM's save/restore at a function that is not in configuration space.
//!
//! [`BifStaticError`] makes both unencodable. ⊘ Both are stated as properties of *this
//! port*: a port that grows a C2C link or a second PCI function grows the right to say
//! `true`, and the check becomes a chip-row predicate at that point.
//!
//! ⚠ `[measured]` in `cap1b` the guest asks this control at `rpc.sequence` **14** (txn 990,
//! `paylen 44` = 40 header + 4 params), inside the replay's closure limit of 1028 — so
//! `crates/kayfabe-crec/tests/cap1b_differential.rs` exercises this reply.

/// `NV2080_CTRL_CMD_INTERNAL_BIF_GET_STATIC_INFO`
/// (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080internal.h:2176`).
pub const NV2080_CTRL_CMD_INTERNAL_BIF_GET_STATIC_INFO: u32 = 0x2080_0aac;

/// Byte offset of `bPcieGen4Capable`.
pub const PCIE_GEN4_CAPABLE_OFF: usize = 0;

/// Byte offset of `bIsC2CLinkUp`.
pub const C2C_LINK_UP_OFF: usize = 1;

/// Byte offset of `bIsDeviceMultiFunction`.
pub const DEVICE_MULTI_FUNCTION_OFF: usize = 2;

/// Byte offset of `bGcxPmuCfgSpaceRestore`.
pub const GCX_PMU_CFG_SPACE_RESTORE_OFF: usize = 3;

/// `sizeof(NV2080_CTRL_INTERNAL_BIF_GET_STATIC_INFO_PARAMS)` — four `NvBool`s, no padding
/// (`ogkm-580: ctrl2080internal.h:2179-2184`).
pub const BIF_STATIC_INFO_PARAMS_SIZE: usize = GCX_PMU_CFG_SPACE_RESTORE_OFF + 1;

/// ★★ **What this device says about its bus interface.**
///
/// ⊘ No `Default`: all-false is both the right answer and what a refusal leaves behind, so
/// the only thing that makes a served answer a *statement* is that someone wrote it down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BifStaticRow {
    /// `bPcieGen4Capable`. Carried verbatim; nothing in the open 580 tree reads the
    /// property it sets. See this module's docs.
    pub pcie_gen4_capable: bool,
    /// `bIsC2CLinkUp` — see [`BifStaticError::C2cLinkUpWithoutC2cPlane`].
    pub c2c_link_up: bool,
    /// `bIsDeviceMultiFunction` — see
    /// [`BifStaticError::MultiFunctionWithoutSecondFunction`].
    pub device_multi_function: bool,
    /// `bGcxPmuCfgSpaceRestore` — whether the PMU restores configuration space across a GCx
    /// cycle. This port serves no power-state transition, so the honest value is `false`.
    pub gcx_pmu_cfg_space_restore: bool,
}

/// Why the reply could not be encoded. Each variant stands in front of a specific guest-side
/// path that would then run against hardware this port does not present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BifStaticError {
    /// ★★ **A chip-to-chip link that is not there.**
    ///
    /// `kmemsysStateInitLocked_IMPL` branches on `PDB_PROP_KBIF_IS_C2C_LINK_UP` to build a
    /// coherent C2C CPU mapping of framebuffer (`ogkm-580: kern_mem_sys.c:168, 342`).
    C2cLinkUpWithoutC2cPlane,
    /// ★★ **A second PCI function that is not in configuration space.**
    ///
    /// `_kbifSavePcieConfigRegisters` saves and restores function 1 through
    /// `xveRegmapRef[1]` when the property is set (`ogkm-580: kernel_bif_gm107.c:430-441`).
    MultiFunctionWithoutSecondFunction,
}

impl core::fmt::Display for BifStaticError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::C2cLinkUpWithoutC2cPlane => write!(
                f,
                "bIsC2CLinkUp is true; RM then builds a coherent C2C mapping of framebuffer \
                 (kern_mem_sys.c:168,342) and this device has no chip-to-chip link"
            ),
            Self::MultiFunctionWithoutSecondFunction => write!(
                f,
                "bIsDeviceMultiFunction is true; RM then saves and restores configuration \
                 space for function 1 through xveRegmapRef[1] (kernel_bif_gm107.c:430-441) \
                 and this device publishes one function"
            ),
        }
    }
}

impl core::error::Error for BifStaticError {}

/// Encode `NV2080_CTRL_INTERNAL_BIF_GET_STATIC_INFO_PARAMS` from a chip's row.
///
/// # Errors
///
/// Both variants of [`BifStaticError`]; see each for the guest-side path it stands in front
/// of.
pub fn encode_bif_static_info(row: &BifStaticRow) -> Result<Vec<u8>, BifStaticError> {
    if row.c2c_link_up {
        return Err(BifStaticError::C2cLinkUpWithoutC2cPlane);
    }
    if row.device_multi_function {
        return Err(BifStaticError::MultiFunctionWithoutSecondFunction);
    }
    let mut params = vec![0u8; BIF_STATIC_INFO_PARAMS_SIZE];
    params[PCIE_GEN4_CAPABLE_OFF] = u8::from(row.pcie_gen4_capable);
    params[C2C_LINK_UP_OFF] = u8::from(row.c2c_link_up);
    params[DEVICE_MULTI_FUNCTION_OFF] = u8::from(row.device_multi_function);
    params[GCX_PMU_CFG_SPACE_RESTORE_OFF] = u8::from(row.gcx_pmu_cfg_space_restore);
    Ok(params)
}
