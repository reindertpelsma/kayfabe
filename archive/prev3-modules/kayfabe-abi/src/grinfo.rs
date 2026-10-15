//! `NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_INFO` (`0x20800a2a`) — GR's **legacy info
//! list**: 58 `(index, data)` pairs per engine, and the one this port refused for four
//! rungs on an argument that a boot then falsified.
//!
//! # ★★★ Why this control stopped being optional
//!
//! [`crate::grstatic`]'s header records `0x20800a2a` as one of the *non*-mandatory nine:
//! `kgraphicsLoadStaticInfo_KERNEL` takes its status into a bare `if (status == NV_OK)`
//! with **no `else`** (`ogkm-580: src/nvidia/src/kernel/gpu/gr/kernel_graphics.c:1234`), so
//! refusing it leaves `pGrInfo` **`NULL`** rather than dangling and the very next control
//! overwrites `status`. That reading is correct and it is still not the whole story: it
//! describes what happens *at the call site*, and the consumer is somewhere else entirely.
//!
//! `[measured]` run `fmb1`, a stock 580.159.04 guest at `93191ee`
//! (`/workspace/bench/run_fmb1_dmesg.log`, and `docs/design/boot_measured_2026_08_01.md`):
//!
//! ```text
//! NVRM: Assertion failed: kgrmgrGetLegacyKGraphicsStaticInfo(...)->pGrInfo != NULL
//!                                                             @ kernel_fifo.c:2789
//! NVRM: Assertion failed: numMax == numFree && numMax != 0
//!                                                             @ kernel_channel_group_api.c:913
//! NVRM: GPU 0000:00:03.0: RmInitAdapter failed! (0x25:0x40:1249)
//! ```
//!
//! `kfifoGetMaxSubcontextFromGr_KERNEL` (`ogkm-580: kernel_fifo.c:2784-2792`) asserts
//! `pGrInfo != NULL` and **returns 0** when it fails — `NV_ASSERT_OR_RETURN` with a value,
//! not a status. Zero maximum subcontexts then reaches
//! `kchangrpapiSetLegacyMode`'s `numMax != 0` (`kernel_channel_group_api.c:913`) as
//! `NV_ERR_INVALID_STATE`, which `gpu.c:3440` does not swallow.
//!
//! ★★ So this is the **third** instance of one shape and it is worth naming as a shape: a
//! refusal that the *caller* tolerates and a *stranger* does not. The caller's `if (status
//! == NV_OK)` is a real observation about `kernel_graphics.c` and it says nothing at all
//! about `kernel_fifo.c`. Reading the call site is necessary and it has now been
//! insufficient three times (`0x20802a08`, `0x20800a1f`, this).
//!
//! # ★★★ Where the 3712 bytes come from: a REAL GA106, not the oracle
//!
//! `[measured]` 2026-08-01 on an NVIDIA GeForce RTX 3060 (GA106),
//! `GPU-e28d7776-e4f9-704b-d392-d46f187343f8`, vast instance `46494693`, host driver
//! **NVIDIA open 580.159.04** — the matching source (`research_clones/ogkm-580.159.04`, tag
//! `b81d58e`) rebuilt with a chunked whole-body print at `rpcRmApiControl_GSP`'s reply and
//! `RmInitAdapter` driven by opening `/dev/nvidia0`. The transcript is committed at
//! `traces/real_ga106/rpc_bodies_real_ga106.txt`; the control appears **twice** in one run
//! and the two bodies are byte-identical.
//!
//! ⊘ The reason it had to be asked of hardware rather than read out of the C artifact is
//! recorded in [`crate::oracle`]: the artifact's `mode2_initctrl_ga106.h` is a **capture**,
//! and a capture is only evidence where it captured something.
//!
//! ★★ Here that same measurement **corroborates** the artifact rather than falsifying it.
//! `0x20800a2a`'s captured row is `{0x20800a2au, 0x0u, 3712u, 3712u, ctl_20800a2a}`
//! (`C: src/qemu/mode2_initctrl_ga106.h:6219 = 0x20800a2a`) — a *full* body, `dlen == psize` — and it is
//! **identical to hardware for all 3712 bytes**. That is the class result stated the other
//! way round: rows the C actually captured are right, rows it recorded empty say nothing.
//!
//! # ★ This module is a table of NAMED indices, and the names are what make it a test
//!
//! The reply is `engineInfo[8].infoList[58]` of `{ NvU32 index; NvU32 data; }`
//! (`ogkm-580: ctrl2080internal.h:408-420`), and RM reads it **by array position**:
//! `infoList[NV0080_CTRL_GR_INFO_INDEX_MAX_SUBCONTEXT_COUNT].data` indexes with the
//! constant, it does not search for it. ⚠ One caller *does* search — `gpuGetGrInfo`-style,
//! `if (pGrInfo->infoList[i].index == index)` at `ogkm-580: gpu.c:6279-6280` — so **both**
//! the position and the `index` field are load-bearing, and they must agree. That is
//! [`GrInfoProfile::encode`]'s first invariant rather than a comment: entry `i` carries
//! `index = i`.
//!
//! ⊘ **What is NOT derived, and is not pretended to be.** These are litter constants —
//! `LITTER_NUM_MXBAR_FBP_PORTS` is a fact about a crossbar, not a function of anything this
//! port models. They are transcribed from the measurement above, one named constant each.
//! What *is* derived is the cross-check: nine of the 58 are the same silicon
//! [`crate::grstatic`] already describes through a different control, and
//! [`GrInfoProfile::validate_against`] refuses a pair that disagrees. Two hand-written
//! descriptions of one chip is the drift that makes a fixture comparison a tautology.
//!
//! # ⊘ What this module does NOT establish
//!
//! - ⊘ **It is GA106 and nothing else.** No AD10x or GH100 part was asked. A chip row that
//!   wants this control must carry its own measured table; borrowing this one would be
//!   stating 58 unmeasured numbers at once.
//! - ⊘ **It does not establish that the boot gets past `kernel_fifo.c:2789`** — only what
//!   the reply must be when it is asked. The rung that follows this one is what says.
//! - ⊘ **Seven of the eight engine rows are zero, and that is the measurement, not a
//!   simplification.** A real GA106 answers `engineInfo[1..8]` all-zero because it has one
//!   GR engine and MIG is off.

use crate::grstatic::{GR_MAX_ENGINES, GrStaticProfile};

/// `NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_INFO`
/// (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080internal.h:422`).
pub const NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_INFO: u32 = 0x2080_0a2a;

/// `NV0080_CTRL_GR_INFO_MAX_SIZE` — `NV0080_CTRL_GR_INFO_INDEX_MAX + 1`
/// (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl0080/ctrl0080gr.h:168-169`).
pub const GR_INFO_MAX_SIZE: usize = 0x3a;

/// `sizeof(NV2080_CTRL_INTERNAL_GR_INFO)` — `{ NvU32 index; NvU32 data; }`
/// (`ogkm-580: ctrl2080internal.h:408-411`).
pub const GR_INFO_ENTRY_SIZE: usize = 8;

/// `sizeof(NV2080_CTRL_INTERNAL_STATIC_KGR_GET_INFO_PARAMS)` — `8 * 58 * 8` = 3712, and the
/// `psize` a real GA106 answers with (`traces/real_ga106/rpc_bodies_real_ga106.txt`).
pub const KGR_GET_INFO_PARAMS_SIZE: usize = GR_MAX_ENGINES * GR_INFO_MAX_SIZE * GR_INFO_ENTRY_SIZE;

// ---------------------------------------------------------------------------------------
// The index names — `ogkm-580: ctrl0080gr.h:101-160`, in declaration order
// ---------------------------------------------------------------------------------------

/// `NV0080_CTRL_GR_INFO_INDEX_SHADER_PIPE_COUNT` — the **enabled** GPC count, not the
/// litter maximum. Cross-checked against [`GrStaticProfile`]'s GPC rows.
pub const IDX_SHADER_PIPE_COUNT: usize = 0x07;
/// `NV0080_CTRL_GR_INFO_INDEX_SHADER_PIPE_SUB_COUNT` — the enabled TPC count.
pub const IDX_SHADER_PIPE_SUB_COUNT: usize = 0x09;
/// `NV0080_CTRL_GR_INFO_INDEX_SM_VERSION` — `0x806` is SM 8.6, which is what GA106 is.
pub const IDX_SM_VERSION: usize = 0x0c;
/// `NV0080_CTRL_GR_INFO_INDEX_MAX_WARPS_PER_SM`.
pub const IDX_MAX_WARPS_PER_SM: usize = 0x0d;
/// `NV0080_CTRL_GR_INFO_INDEX_MAX_THREADS_PER_WARP`.
pub const IDX_MAX_THREADS_PER_WARP: usize = 0x0e;
/// `NV0080_CTRL_GR_INFO_INDEX_LITTER_NUM_GPCS` — ★ the **family** maximum, and a *bound*:
/// `kgrmgrGetLegacyTpcMask_IMPL` rejects every `gpcId >= maxNumGpcs`
/// (`ogkm-580: kernel_graphics_manager.c:625-626`), so a zero here silently answers an
/// empty TPC mask for every GPC.
pub const IDX_LITTER_NUM_GPCS: usize = 0x14;
/// `NV0080_CTRL_GR_INFO_INDEX_GPU_CORE_COUNT` — cross-checked as `sm_count * 128`.
pub const IDX_GPU_CORE_COUNT: usize = 0x1d;
/// `NV0080_CTRL_GR_INFO_INDEX_LITTER_NUM_SM_PER_TPC`.
pub const IDX_LITTER_NUM_SM_PER_TPC: usize = 0x20;
/// `NV0080_CTRL_GR_INFO_INDEX_RT_CORE_COUNT` — one per SM on Ampere.
pub const IDX_RT_CORE_COUNT: usize = 0x22;
/// `NV0080_CTRL_GR_INFO_INDEX_TENSOR_CORE_COUNT` — four per SM on Ampere.
pub const IDX_TENSOR_CORE_COUNT: usize = 0x23;
/// ★★★ `NV0080_CTRL_GR_INFO_INDEX_MAX_SUBCONTEXT_COUNT` — **the entry that ended run
/// `fmb1`**, read by `kfifoGetMaxSubcontextFromGr_KERNEL` (`ogkm-580: kernel_fifo.c:2792`)
/// and again as a golden-context-buffer multiplier (`kernel_graphics.c:1765-1772`).
pub const IDX_MAX_SUBCONTEXT_COUNT: usize = 0x2c;
/// `NV0080_CTRL_GR_INFO_INDEX_MAX_LEGACY_SUBCONTEXT_COUNT`.
pub const IDX_MAX_LEGACY_SUBCONTEXT_COUNT: usize = 0x2d;
/// `NV0080_CTRL_GR_INFO_INDEX_MAX_PER_ENGINE_SUBCONTEXT_COUNT`.
pub const IDX_MAX_PER_ENGINE_SUBCONTEXT_COUNT: usize = 0x2e;
/// `NV0080_CTRL_GR_INFO_INDEX_GFX_CAPABILITIES`, read at `kernel_graphics.c:1603`.
pub const IDX_GFX_CAPABILITIES: usize = 0x34;
/// `NV0080_CTRL_GR_INFO_INDEX_MAX_MIG_ENGINES`.
pub const IDX_MAX_MIG_ENGINES: usize = 0x35;
/// `NV0080_CTRL_GR_INFO_INDEX_MAX_PARTITIONABLE_GPCS`.
pub const IDX_MAX_PARTITIONABLE_GPCS: usize = 0x36;
/// `NV0080_CTRL_GR_INFO_INDEX_LITTER_MIN_SUBCTX_PER_SMC_ENG` — a **multiplier**:
/// `*pVeidCount = gpcCount * veidStepSize` (`ogkm-580: kgrmgr_ga100.c:44`), so a zero here
/// makes every VEID allocation ask for zero VEIDs.
pub const IDX_LITTER_MIN_SUBCTX_PER_SMC_ENG: usize = 0x37;

/// CUDA cores per SM on Ampere — the multiplier that turns an SM count into
/// [`IDX_GPU_CORE_COUNT`]. Not an ogkm constant: it is the arithmetic that makes the
/// `3584` a *checkable* number rather than a transcribed one. `[measured]` 2026-08-01 on
/// an RTX 3060, `traces/real_ga106/rpc_bodies_real_ga106.txt`, and the check is
/// `tests::the_ga106_row_agrees_with_the_ga106_gr_static_geometry`: 28 SM x 128.
pub const AMPERE_CORES_PER_SM: u32 = 128;
/// Tensor cores per SM on Ampere — the multiplier for [`IDX_TENSOR_CORE_COUNT`].
pub const AMPERE_TENSOR_CORES_PER_SM: u32 = 4;

/// Why a GR info list could not be encoded.
///
/// ★ Every variant is a value whose *zero* is read by RM as a number rather than rejected
/// as an error — the [`crate::fmbsize`] shape. None of them is a defensive check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrInfoError {
    /// ★★★ [`IDX_MAX_SUBCONTEXT_COUNT`] is zero — **the exact value run `fmb1` died of**.
    /// `kfifoGetMaxSubcontextFromGr_KERNEL` returns it verbatim and
    /// `kchangrpapiSetLegacyMode` asserts `numMax != 0`
    /// (`ogkm-580: kernel_channel_group_api.c:913`). Answering it would rebuild the wall
    /// while *looking* served, which is strictly worse than refusing.
    MaxSubcontextCountZero,
    /// [`IDX_LITTER_NUM_GPCS`] is zero — every `gpcId < maxNumGpcs` bound fails and GR
    /// answers an empty TPC mask for GPCs it has just published in another control.
    LitterNumGpcsZero,
    /// [`IDX_LITTER_MIN_SUBCTX_PER_SMC_ENG`] is zero — `gpcCount * veidStepSize` is zero
    /// VEIDs (`ogkm-580: kgrmgr_ga100.c:38-44`).
    VeidStepSizeZero,
    /// The two descriptions of one chip disagree: `field` names which.
    ///
    /// ⊘ Not a style check. [`crate::grstatic`] publishes the GPC and TPC geometry through
    /// `0x20800a1f`/`0x20800a26`/`0x20800a22`, and this control publishes it again. RM
    /// reads both.
    DisagreesWithGrStatic {
        /// The index whose value contradicts the [`GrStaticProfile`].
        index: usize,
        /// What this profile says.
        info: u32,
        /// What the GR static geometry says.
        derived: u32,
    },
}

/// GR's legacy info list for **one** GR engine — 58 `data` words, positionally indexed.
///
/// ★ `data` only. The `index` field is not stored because it is not free: RM indexes by
/// position *and* one caller searches by `index`, so the only correct table is the one
/// where they agree, and storing both would make disagreement expressible.
#[derive(Debug, Clone, Copy)]
pub struct GrInfoProfile {
    /// `infoList[i].data`, for `i` in `0..58`.
    pub data: [u32; GR_INFO_MAX_SIZE],
}

impl GrInfoProfile {
    /// The invariants RM's own readers rest on.
    ///
    /// # Errors
    /// The three zero variants of [`GrInfoError`].
    pub fn validate(&self) -> Result<(), GrInfoError> {
        if self.data[IDX_MAX_SUBCONTEXT_COUNT] == 0 {
            return Err(GrInfoError::MaxSubcontextCountZero);
        }
        if self.data[IDX_LITTER_NUM_GPCS] == 0 {
            return Err(GrInfoError::LitterNumGpcsZero);
        }
        if self.data[IDX_LITTER_MIN_SUBCTX_PER_SMC_ENG] == 0 {
            return Err(GrInfoError::VeidStepSizeZero);
        }
        Ok(())
    }

    /// ★★ The cross-check: nine entries restate geometry [`crate::grstatic`] publishes
    /// through three other controls, and RM reads both descriptions.
    ///
    /// ⊘ It deliberately does **not** check the litter constants. `LITTER_NUM_GPCS = 7` on
    /// a three-GPC part is not a contradiction — it is the *family* maximum — and a check
    /// that "corrected" it would be inventing a relationship. Only the entries that name
    /// the same quantity are compared.
    ///
    /// # Errors
    /// [`GrInfoError::DisagreesWithGrStatic`] naming the first index that disagrees, plus
    /// anything [`GrStaticProfile::validate`] itself rejects, reported as a disagreement at
    /// [`IDX_SHADER_PIPE_COUNT`] because a geometry that does not validate cannot be
    /// compared against.
    pub fn validate_against(&self, gr: &GrStaticProfile) -> Result<(), GrInfoError> {
        let sm_count = gr
            .num_sm()
            .map_err(|_| GrInfoError::DisagreesWithGrStatic {
                index: IDX_SHADER_PIPE_COUNT,
                info: self.data[IDX_SHADER_PIPE_COUNT],
                derived: 0,
            })?;
        let sm_count = u32::from(sm_count);
        let tpc_count = u32::try_from(gr.tpcs.len()).unwrap_or(u32::MAX);
        let gpc_count = u32::try_from(gr.gpcs.len()).unwrap_or(u32::MAX);

        // (index, what the GR static geometry says it must be)
        let pairs = [
            (IDX_SHADER_PIPE_COUNT, gpc_count),
            (IDX_SHADER_PIPE_SUB_COUNT, tpc_count),
            (IDX_LITTER_NUM_SM_PER_TPC, u32::from(gr.sms_per_tpc)),
            (IDX_GPU_CORE_COUNT, sm_count * AMPERE_CORES_PER_SM),
            (IDX_RT_CORE_COUNT, sm_count),
            (IDX_TENSOR_CORE_COUNT, sm_count * AMPERE_TENSOR_CORES_PER_SM),
        ];
        for (index, derived) in pairs {
            if self.data[index] != derived {
                return Err(GrInfoError::DisagreesWithGrStatic {
                    index,
                    info: self.data[index],
                    derived,
                });
            }
        }
        Ok(())
    }

    /// `NV2080_CTRL_INTERNAL_STATIC_KGR_GET_INFO_PARAMS` — engine 0 from this profile, the
    /// other seven zero, which is what a real GA106 answers.
    ///
    /// ★ Entry `i` is written as `(index = i, data = self.data[i])`. The `index` field is
    /// **not** an input, so the position/`index` agreement `ogkm-580: gpu.c:6279` depends on
    /// cannot be broken by a chip row.
    ///
    /// # Errors
    /// [`GrInfoError`] — see [`GrInfoProfile::validate`].
    pub fn encode(&self) -> Result<Vec<u8>, GrInfoError> {
        self.validate()?;
        let mut out = vec![0u8; KGR_GET_INFO_PARAMS_SIZE];
        for (i, data) in self.data.iter().enumerate() {
            let at = i * GR_INFO_ENTRY_SIZE;
            let index = u32::try_from(i).unwrap_or(u32::MAX);
            out[at..at + 4].copy_from_slice(&index.to_le_bytes());
            out[at + 4..at + 8].copy_from_slice(&data.to_le_bytes());
        }
        Ok(out)
    }
}

/// GA106 — `[measured]` 2026-08-01 on an RTX 3060 running open 580.159.04; the whole body
/// is at `traces/real_ga106/rpc_bodies_real_ga106.txt`.
///
/// ⊘ Written as a positional array with the load-bearing entries named in the comments
/// rather than as 58 `pub const`s: the array *is* the wire order, and a table whose order
/// is only implied by 58 separate names is a table whose order nothing checks.
pub const GA106_GR_INFO: GrInfoProfile = GrInfoProfile {
    data: [
        1,     // 0x00 MAXCLIPS
        0,     // 0x01 MIN_ATTRS_BUG_261894
        0,     // 0x02 XBUF_MAX_PSETS_PER_BANK
        512,   // 0x03 BUFFER_ALIGNMENT
        0,     // 0x04 SWIZZLE_ALIGNMENT
        16,    // 0x05 VERTEX_CACHE_SIZE
        0,     // 0x06 VPE_COUNT
        3,     // 0x07 SHADER_PIPE_COUNT        ← GPC count, cross-checked
        28,    // 0x08 THREAD_STACK_SCALING_FACTOR
        14,    // 0x09 SHADER_PIPE_SUB_COUNT    ← TPC count, cross-checked
        0,     // 0x0a SM_REG_BANK_COUNT
        0,     // 0x0b SM_REG_BANK_REG_COUNT
        0x806, // 0x0c SM_VERSION               ← SM 8.6
        48,    // 0x0d MAX_WARPS_PER_SM
        32,    // 0x0e MAX_THREADS_PER_WARP
        160,   // 0x0f GEOM_GS_OBUF_ENTRIES
        160,   // 0x10 GEOM_XBUF_ENTRIES
        32,    // 0x11 FB_MEMORY_REQUEST_GRANULARITY
        32,    // 0x12 HOST_MEMORY_REQUEST_GRANULARITY
        0,     // 0x13 MAX_SP_PER_SM
        7,     // 0x14 LITTER_NUM_GPCS          ← family max, a BOUND
        6,     // 0x15 LITTER_NUM_FBPS
        4,     // 0x16 LITTER_NUM_ZCULL_BANKS
        6,     // 0x17 LITTER_NUM_TPC_PER_GPC
        1,     // 0x18 LITTER_NUM_MIN_FBPS
        8,     // 0x19 LITTER_NUM_MXBAR_FBP_PORTS
        1,     // 0x1a TIMESLICE_ENABLED
        6,     // 0x1b LITTER_NUM_FBPAS
        3,     // 0x1c LITTER_NUM_PES_PER_GPC
        3584,  // 0x1d GPU_CORE_COUNT           ← 28 SM * 128, cross-checked
        2,     // 0x1e LITTER_NUM_TPCS_PER_PES
        8,     // 0x1f LITTER_NUM_MXBAR_HUB_PORTS
        2,     // 0x20 LITTER_NUM_SM_PER_TPC    ← cross-checked
        2,     // 0x21 LITTER_NUM_HSHUB_FBP_PORTS
        28,    // 0x22 RT_CORE_COUNT            ← 1 per SM, cross-checked
        112,   // 0x23 TENSOR_CORE_COUNT        ← 4 per SM, cross-checked
        1,     // 0x24 LITTER_NUM_GRS
        12,    // 0x25 LITTER_NUM_LTCS
        8,     // 0x26 LITTER_NUM_LTC_SLICES
        1,     // 0x27 LITTER_NUM_GPCMMU_PER_GPC
        2,     // 0x28 LITTER_NUM_LTC_PER_FBP
        2,     // 0x29 LITTER_NUM_ROP_PER_GPC
        8,     // 0x2a FAMILY_MAX_TPC_PER_GPC
        1,     // 0x2b LITTER_NUM_FBPA_PER_FBP
        64,    // 0x2c MAX_SUBCONTEXT_COUNT     ★★★ the entry run fmb1 died without
        2,     // 0x2d MAX_LEGACY_SUBCONTEXT_COUNT
        64,    // 0x2e MAX_PER_ENGINE_SUBCONTEXT_COUNT
        0,     // 0x2f — no name in ctrl0080gr.h; hardware answers zero
        0,     // 0x30 — no name in ctrl0080gr.h; hardware answers zero
        0,     // 0x31 — no name in ctrl0080gr.h; hardware answers zero
        4,     // 0x32 LITTER_NUM_SLICES_PER_LTC
        0,     // 0x33 LITTER_NUM_GFXC_SMC_ENGINES (aliased NV..._DUMMY)
        15,    // 0x34 GFX_CAPABILITIES
        1,     // 0x35 MAX_MIG_ENGINES
        7,     // 0x36 MAX_PARTITIONABLE_GPCS
        9,     // 0x37 LITTER_MIN_SUBCTX_PER_SMC_ENG ← a MULTIPLIER
        0,     // 0x38 LITTER_NUM_GPCS_PER_DIELET
        0,     // 0x39 LITTER_MAX_NUM_SMC_ENGINES_PER_DIELET
    ],
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_size_is_the_size_hardware_answered() {
        // ★ Derived from three ogkm constants, then compared against the `psize` on the
        // wire — not typed in. 8 * 58 * 8.
        assert_eq!(KGR_GET_INFO_PARAMS_SIZE, 3712);
    }

    #[test]
    fn every_entry_carries_its_own_position_as_its_index() {
        // ★★ The invariant `ogkm-580: gpu.c:6279-6280` searches on, checked over the RAW
        // bytes rather than through any accessor.
        let body = GA106_GR_INFO.encode().expect("the GA106 row validates");
        assert_eq!(body.len(), KGR_GET_INFO_PARAMS_SIZE);
        for i in 0..GR_INFO_MAX_SIZE {
            let at = i * GR_INFO_ENTRY_SIZE;
            let index = u32::from_le_bytes(body[at..at + 4].try_into().unwrap());
            assert_eq!(
                index as usize, i,
                "entry {i} does not carry its own position"
            );
        }
    }

    #[test]
    fn the_seven_other_engine_rows_are_zero() {
        let body = GA106_GR_INFO.encode().expect("the GA106 row validates");
        let row = GR_INFO_MAX_SIZE * GR_INFO_ENTRY_SIZE;
        assert!(
            body[row..].iter().all(|&b| b == 0),
            "engineInfo[1..8] must be zero — a real GA106 has one GR engine"
        );
    }

    #[test]
    fn a_zero_max_subcontext_count_is_refused_by_name() {
        // ★★★ The wall, expressed as a value. Not "some error" — the named one.
        let mut p = GA106_GR_INFO;
        p.data[IDX_MAX_SUBCONTEXT_COUNT] = 0;
        assert_eq!(p.encode(), Err(GrInfoError::MaxSubcontextCountZero));
    }

    #[test]
    fn the_other_two_load_bearing_zeros_are_refused_by_name() {
        let mut p = GA106_GR_INFO;
        p.data[IDX_LITTER_NUM_GPCS] = 0;
        assert_eq!(p.encode(), Err(GrInfoError::LitterNumGpcsZero));

        let mut p = GA106_GR_INFO;
        p.data[IDX_LITTER_MIN_SUBCTX_PER_SMC_ENG] = 0;
        assert_eq!(p.encode(), Err(GrInfoError::VeidStepSizeZero));
    }

    #[test]
    fn the_ga106_row_agrees_with_the_ga106_gr_static_geometry() {
        // ★★ The cross-check that makes the 58 numbers more than a transcription: six of
        // them are the same silicon `grstatic` publishes through three other controls.
        GA106_GR_INFO
            .validate_against(&crate::grstatic::GA106_GR_STATIC)
            .expect("one chip, two descriptions, and they must agree");
    }

    #[test]
    fn a_disagreement_with_the_gr_static_geometry_is_refused_and_names_the_index() {
        let mut p = GA106_GR_INFO;
        p.data[IDX_RT_CORE_COUNT] = 27;
        assert_eq!(
            p.validate_against(&crate::grstatic::GA106_GR_STATIC),
            Err(GrInfoError::DisagreesWithGrStatic {
                index: IDX_RT_CORE_COUNT,
                info: 27,
                derived: 28,
            })
        );
    }
}
