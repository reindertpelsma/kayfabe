//! `NV2080_CTRL_CMD_GRMGR_GET_GR_FS_INFO` (`0x20803801`) — ★★★ the graphics floorsweeping
//! **batch** query, and the first control this port serves whose error shape is
//! **per-item rather than per-call**.
//!
//! ## ★★★ The property that decides the whole design, and it is the OPPOSITE of `FB_GET_INFO_V2`
//!
//! `ogkm-580: ctrl2080grmgr.h:42-50`, in as many words:
//!
//! > *"This control call works as a batched query interface … If there is any error in
//! > `NV2080_CTRL_GRMGR_GET_GR_FS_INFO_PARAMS`, we will immediately fail the call. However,
//! > if there is an error in the query-specific calls, we will **log the error and march
//! > on**."*
//!
//! So a **structural** fault (a `numQueries` past [`MAX_QUERIES`]) fails the whole control,
//! and a **per-query** fault is written into that query's own `status` word while the call
//! still returns `NV_OK`. ⊘ [`crate::fbinfo`]'s rule — *"one refused index fails all of
//! them"* — is exactly wrong here, and copying it across would have refused a call a real
//! GA106 answers.
//!
//! ⚠ **And that cuts both ways, which is the trap in this file.** A per-query `status` of
//! `NV_ERR_NOT_SUPPORTED` on a query type real hardware *answers* is a wrong answer that
//! **arrives inside an `NV_OK` reply** — invisible to the served/unserviced ledgers, which
//! is `refusal_invisible_in_the_ledger` with a new carrier. ⇒ This module refuses per-query
//! **only** where a refusal is the *documented hardware behaviour*, and refuses the **whole
//! call** — loudly, into the ledger — for every query type it merely does not model. See
//! [`GrFsQuery::answer`].
//!
//! ## Routing — ⊘ there is NO id translation here, and that is worth stating
//!
//! §14.33's rung turned on `0x20802a0a` forwarding to a *different* id. This one does not.
//! Its export flags are `0x10248` (`ogkm-580: g_subdevice_nvoc.c:9520-9534`) =
//! `NON_PRIVILEGED(0x8) | ROUTE_TO_PHYSICAL(0x40) | ROUTE_TO_VGPU_HOST(0x200) |
//! GSP_PLUGIN_FOR_VGPU_GSP(0x10000)`, and `ROUTE_TO_PHYSICAL` makes
//! `NVOC_EXPORTED_METHOD_DISABLED_BY_FLAG` true (`control.h:159-161`), so
//! `subdeviceCtrlCmdGrmgrGetGrFsInfo_IMPL`'s function pointer is compiled to `NULL` and
//! **the body is not in the open tree at all**. `rmresControl_Prologue_IMPL`
//! (`resource.c:255-291`) RPCs `pParams->cmd` **unmodified** with the same 1928-byte buffer,
//! and `rs_resource.c:191-200` then skips the handler. ⇒ Whatever this port writes is what
//! libcuda sees, byte for byte; CPU-RM contributes nothing before or after.
//!
//! ★ Note it carries `NON_PRIVILEGED`, so unlike [`crate::cecaps`]'s `0x20802a0b` this one
//! **is** probeable from usermode — but it is a query list with `[IN]` fields, so a
//! `0xCD`-seeded `--probe-ctrl` would ask query type `0xCDCD`. Any sweep must *set*
//! `numQueries`, `queryType` and the per-type input words and seed only the `[OUT]` region.
//!
//! ## The layout, and how the stride was established
//!
//! `NV2080_CTRL_GRMGR_GET_GR_FS_INFO_PARAMS` (`ogkm-580: ctrl2080grmgr.h:264-268`) is
//! `NvU16 numQueries; NvU8 reserved[6]; QUERY queries[96];` — total `8 + 96×20` = **1928**.
//!
//! ⚠ **`NV2080_CTRL_GRMGR_GR_FS_INFO_QUERY_MAX_SIZE = 32` (`:66`) is NOT the element size.**
//! It is an aspirational bound; the element is **20** bytes. A reader who takes the named
//! constant gets 8 + 96×32 = 3080 and a struct that does not exist.
//!
//! ★ `[measured 2026-08-09]` the stride was established from the wire *before* the header
//! was read, and the two agree. `traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt:64` is
//! a **complete** 1928-byte record (the interposer's `TRUNC` marker is absent), and
//! byte-diffing its `in=` against its `out=` shows **exactly two changed bytes in the whole
//! struct**, at offsets **40** and **60**. Only a 20-byte stride based at 8 reads those as
//! one field of one repeated record:
//!
//! | stride | q0 / q1 / q2 words | verdict |
//! |---|---|---|
//! | **20** | tag `2` at +0 in all three, index `0,1,2` at +8, the only changed word at +12 | ★ three instances of one record |
//! | 24 | the tag lands in a different slot each time | ⊘ incoherent |
//! | 16 | likewise | ⊘ incoherent |
//!
//! ⊘ Size arithmetic alone does **not** discriminate — `1920` divides by 16, 20 and 24 —
//! and choosing the divisor that "works" is `two_encodings_agreeing_on_the_first_values`.
//! What discriminates is coherence of the repeated record, and the header then confirms it.
//!
//! ## What libcuda actually asks, and what it gets
//!
//! `numQueries = 3`, all three [`query_type::CHIPLET_GPC_MAP`], with `gpcId` `0, 1, 2` and
//! `chipletGpcMap` answered `0, 1, 2`. ⊘ **Two of the three, not three:** query 0 is asked
//! for `gpcId = 0` and answered `0`, which is indistinguishable from *not written* because
//! libcuda hands RM a zeroed buffer. The identity is supported by q1 and q2 and merely
//! consistent with q0.
//!
//! ⊘⊘ **CORRECTED 2026-09-26 (v3-gpcmask) — the paragraph below was WRONG in its second
//! half, and a real GA106 says so.** `CHIPLET_GPC_MAP` is not *"the `n`-th set bit of
//! `gpcMask`"*: the logical order is the firmware's, and `[measured]` it is not the physical
//! order even on a contiguous mask — `traces/rpctrace_ga106_boot1.bin` (an RTX 3060, 580.159.04)
//! has physical GPC 1 as its four-TPC GPC and `tpcCount[0] = 4`, so logical GPC 0 is physical
//! GPC 1 there. ⇒ The map is now the HOST's own answer to this very control
//! ([`crate::grstatic::GpcRow::physical_id`], asked at realize by `kf_rm::hostquery`), and this
//! module no longer derives it at all. `[measured 2026-09-26, RTX 3060 Ti, 580.159.04, host]`
//! the answers the per-GPC types give, which this module now serves:
//!
//! | type | `[IN] gpcId` | measured |
//! |---|---|---|
//! | `CHIPLET_GPC_MAP` | **logical** | `1,2,3,4,5` for `gpcMask = 0x3e`; `gpcId >= 5` → per-query `0x1f` `NV_ERR_INVALID_ARGUMENT` |
//! | `TPC_MASK` | **logical** | the PHYSICAL TPC mask of that GPC: `e,f,f,f,f` (`tpcMask[]` is `0,e,f,f,f,f`); `>= 5` → `0x1f` |
//! | `PPC_MASK` / `ROP_MASK` | **logical** | `3` each; `>= 5` → `0x1f` |
//! | `GPC_COUNT` / `CHIPLET_SYSPIPE_MASK` / `CHIPLET_GRAPHICS_SYSPIPE_MASK` | — | `5` / `1` / **`0`** |
//!
//! ⊘ SUPERSEDED 2026-09-26 by the correction above — kept as the reasoning it records:
//! ⚠ ★★ **And "identity" is not what this module implements**, because on GA106 the identity
//! and the correct derivation agree and cannot be told apart. `CHIPLET_GPC_MAP` maps a
//! **logical** GPC index to a **physical** (chiplet) GPC id, which is *"the `n`-th set bit of
//! `gpcMask`"*. GA106's mask is `0b111`, contiguous from bit 0, so both readings give
//! `0,1,2`. This module derives from the mask; a floorswept part is where the two would
//! separate, and there the derivation is the right one.

use crate::NV_ERR_NOT_SUPPORTED;
use crate::grstatic::{GpcRow, GrStaticProfile, GrSyspipeMasks};

/// `NV_ERR_INVALID_ARGUMENT` — what a real GPU writes into a per-GPC query's `status` for a
/// `gpcId` at or past the GPC count (`[measured 2026-09-26, RTX 3060 Ti, 580.159.04]`: `0x1f`
/// for `CHIPLET_GPC_MAP`, `TPC_MASK`, `PPC_MASK` and `ROP_MASK` alike).
pub const NV_ERR_INVALID_ARGUMENT: u32 = 0x0000_001f;

/// `NV2080_CTRL_CMD_GRMGR_GET_GR_FS_INFO` — `ogkm-580: ctrl2080grmgr.h:56`.
pub const NV2080_CTRL_CMD_GRMGR_GET_GR_FS_INFO: u32 = 0x2080_3801;

/// `NV2080_CTRL_GRMGR_GR_FS_INFO_MAX_QUERIES` — `ogkm-580: ctrl2080grmgr.h:59`.
pub const MAX_QUERIES: usize = 96;

/// Byte offset of `queries[0]`: `NvU16 numQueries` then `NvU8 reserved[6]`.
pub const QUERIES_OFF: usize = 8;

/// `sizeof(NV2080_CTRL_GRMGR_GR_FS_INFO_QUERY_PARAMS)` — `ogkm-580: ctrl2080grmgr.h:239-256`.
///
/// ⊘ **Not** `NV2080_CTRL_GRMGR_GR_FS_INFO_QUERY_MAX_SIZE` (32), which is a bound and not a
/// size. See the module header.
pub const QUERY_STRIDE: usize = 20;

/// Offset of `queryType` (`NvU16`) within a query element.
pub const QUERY_TYPE_OFF: usize = 0;
/// Offset of the per-query `status` (`NvU32`) within a query element — the `[OUT]` word that
/// makes this control per-item fault tolerant.
pub const QUERY_STATUS_OFF: usize = 4;
/// Offset of the `queryData` union within a query element.
pub const QUERY_DATA_OFF: usize = 8;

/// `sizeof(NV2080_CTRL_GRMGR_GET_GR_FS_INFO_PARAMS)` = `8 + 96 × 20` = 1928, and the
/// `paramSize` the export row advertises (`ogkm-580: g_subdevice_nvoc.c:9529`).
pub const GR_FS_INFO_PARAMS_SIZE: usize = QUERIES_OFF + MAX_QUERIES * QUERY_STRIDE;

/// The thirteen `NV2080_CTRL_GRMGR_GR_FS_INFO_QUERY_*` values —
/// `ogkm-580: ctrl2080grmgr.h:270-282`.
pub mod query_type {
    /// `_INVALID`. Never legal.
    pub const INVALID: u16 = 0;
    /// `_GPC_COUNT` — `[OUT] gpcCount` at `queryData + 0`.
    pub const GPC_COUNT: u16 = 1;
    /// `_CHIPLET_GPC_MAP` — `[IN] gpcId` at `+0`, `[OUT] chipletGpcMap` at `+4`.
    /// ★ The only type libcuda's `cuInit` is measured to ask.
    pub const CHIPLET_GPC_MAP: u16 = 2;
    /// `_TPC_MASK` — `[IN] gpcId` (LOGICAL) at `+0`, `[OUT] tpcMask` (physical TPC bits) at `+4`.
    pub const TPC_MASK: u16 = 3;
    /// `_PPC_MASK` — `[IN] gpcId` (LOGICAL) at `+0`, `[OUT] ppcMask` at `+4`.
    pub const PPC_MASK: u16 = 4;
    /// `_PARTITION_CHIPLET_GPC_MAP` — ⊘ **deprecated and unconditionally refused by RM
    /// itself**: *"This query will return `NV_ERR_NOT_SUPPORTED` since deleting it would
    /// break driver compatibility"* (`ctrl2080grmgr.h:118-120`).
    pub const PARTITION_CHIPLET_GPC_MAP: u16 = 5;
    /// `_CHIPLET_SYSPIPE_MASK` — `[OUT]`. *"Legacy case returns 1 GR"* (`:150`).
    pub const CHIPLET_SYSPIPE_MASK: u16 = 6;
    /// `_PARTITION_CHIPLET_SYSPIPE_IDS` — MIG only (`:167`).
    pub const PARTITION_CHIPLET_SYSPIPE_IDS: u16 = 7;
    /// `_PROFILER_MON_GPC_MASK` — MIG only (`:188`).
    pub const PROFILER_MON_GPC_MASK: u16 = 8;
    /// `_PARTITION_SYSPIPE_ID` — MIG only (`:198`).
    pub const PARTITION_SYSPIPE_ID: u16 = 9;
    /// `_ROP_MASK` — `[IN] gpcId` (LOGICAL) at `+0`, `[OUT] ropMask` at `+4`.
    pub const ROP_MASK: u16 = 10;
    /// `_CHIPLET_GRAPHICS_SYSPIPE_MASK` — `[OUT]`. *"Legacy case returns GR0 if GFX capable,
    /// else 0"* (`:208`).
    pub const CHIPLET_GRAPHICS_SYSPIPE_MASK: u16 = 11;
    /// `_GFX_CAPABLE_GPC_MASK` — MIG only (`:227`).
    pub const GFX_CAPABLE_GPC_MASK: u16 = 12;
}

/// The floorsweeping facts this reply is projected from — ⊘ no new numbers, all of them the
/// rows this port already serves to `INTERNAL_STATIC_KGR_GET_FLOORSWEEPING_MASKS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrFsGeometry<'a> {
    /// `physGfxGpcMask` — which physical GPCs are graphics capable.
    pub gfx_gpc_mask: u32,
    /// ★ One row per **logical** GPC, in logical order — [`GrStaticProfile::gpcs`], the same
    /// rows the floorsweeping control encodes. `CHIPLET_GPC_MAP[i]` is `gpcs[i].physical_id`.
    pub gpcs: &'a [GpcRow],
    /// The host's syspipe words; `None` serves the header's legacy rule.
    pub syspipe_masks: Option<GrSyspipeMasks>,
}

impl<'a> GrFsGeometry<'a> {
    /// The projection of one GR profile — one description of one silicon.
    #[must_use]
    pub fn from_profile(p: &GrStaticProfile) -> GrFsGeometry<'static> {
        GrFsGeometry {
            gfx_gpc_mask: p.gfx_gpc_mask,
            gpcs: p.gpcs,
            syspipe_masks: p.syspipe_masks,
        }
    }

    /// Map a **logical** GPC index to its **physical** (chiplet) GPC id — the row's own
    /// `physical_id`, i.e. the host's answer to this control.
    ///
    /// ⊘ Not "the `logical`-th set bit of `gpcMask`" (the rule until 2026-09-26): a real GA106
    /// whose four-TPC GPC is physical 1 has logical 0 → physical 1 on a contiguous `0b111`.
    #[must_use]
    pub fn physical_gpc(&self, logical: u32) -> Option<u32> {
        self.gpcs.get(logical as usize).map(|g| g.physical_id)
    }
}

/// One decoded query element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrFsQuery {
    /// `queryType`.
    pub query_type: u16,
    /// The first `[IN]` word of `queryData`, for the types that take one.
    pub input: u32,
}

/// What a query resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryAnswer {
    /// `status = NV_OK` and this `u32` written at `queryData + 4` (or `+ 0` for the types
    /// whose only field is the output).
    Data {
        /// Byte offset within `queryData`.
        at: usize,
        /// The value.
        value: u32,
    },
    /// `status` written into the query, no data — ★ and this is the **right answer**, not a
    /// gap: RM itself refuses these (`NV_ERR_NOT_SUPPORTED` for a MIG-only type on a non-MIG
    /// part, [`NV_ERR_INVALID_ARGUMENT`] for a `gpcId` past the GPC count).
    RefusedByHardware {
        /// The per-query status.
        status: u32,
    },
    /// ⊘ This port does not model the query. The **whole call** must be refused.
    Unmodelled,
}

impl GrFsQuery {
    /// Resolve one query against the geometry.
    ///
    /// ## ⊘ Why three outcomes and not two
    ///
    /// The tempting design is two: answer, or write `NV_ERR_NOT_SUPPORTED` into the query's
    /// `status`. It is wrong, and the reason is this control's own tolerance. A per-query
    /// refusal rides inside an `NV_OK` reply, so it reaches **no ledger this port keeps** —
    /// not the unserviced list (the command was served) and not the served list's result
    /// column (the result is `0`). A query type we simply have not modelled would therefore
    /// become a **silent wrong answer** to a guest that a real GA106 answers correctly.
    ///
    /// ⇒ [`QueryAnswer::RefusedByHardware`] is used **only** where the header states RM
    /// refuses on a legacy/non-MIG part, or where a real GPU was measured to refuse (a `gpcId`
    /// past the GPC count). Everything else this port cannot answer returns
    /// [`QueryAnswer::Unmodelled`] and takes the whole control down, which is loud, appears
    /// in the ledger, and costs exactly one boot to find.
    #[must_use]
    pub fn answer(self, geom: &GrFsGeometry<'_>) -> QueryAnswer {
        // ★ The per-GPC types take a LOGICAL `gpcId`; past the GPC count a real GPU writes
        // `NV_ERR_INVALID_ARGUMENT` into the query and marches on (measured, module header).
        let row = geom.gpcs.get(self.input as usize);
        let per_gpc = |value: fn(&GpcRow) -> Option<u32>| match row {
            None => QueryAnswer::RefusedByHardware {
                status: NV_ERR_INVALID_ARGUMENT,
            },
            Some(g) => match value(g) {
                Some(v) => QueryAnswer::Data { at: 4, value: v },
                // ⊘ In range, but the host's word was never measured: whole-call refusal.
                None => QueryAnswer::Unmodelled,
            },
        };
        match self.query_type {
            // `nvPopCount32(gpcMask)` — the same idiom RM uses
            // (`ogkm-580: kernel_graphics_manager.c:1041`); one row per enabled GPC.
            query_type::GPC_COUNT => QueryAnswer::Data {
                at: 0,
                value: u32::try_from(geom.gpcs.len()).unwrap_or(u32::MAX),
            },
            // ★ The one libcuda asks: the host's own logical → physical map.
            query_type::CHIPLET_GPC_MAP => per_gpc(|g| Some(g.physical_id)),
            // ★ Modelled since 2026-09-26: a LOGICAL `gpcId` answered with that GPC's PHYSICAL
            // TPC mask — the row's `tpc_mask`, the word `tpcMask[physical_id]` carries.
            query_type::TPC_MASK => per_gpc(|g| Some(g.tpc_mask)),
            // The host's own words, when realize measured them.
            query_type::PPC_MASK => per_gpc(|g| g.ppc_mask),
            query_type::ROP_MASK => per_gpc(|g| g.rop_mask),
            // *"Legacy case returns 1 GR"* / *"GR0 if GFX capable, else 0"* — the header's
            // rule when the host's words were not measured; the host's words when they were
            // (⚠ a GeForce answers 0 for the second: `GrSyspipeMasks`).
            query_type::CHIPLET_SYSPIPE_MASK => QueryAnswer::Data {
                at: 0,
                value: geom.syspipe_masks.map_or(1, |m| m.syspipe),
            },
            query_type::CHIPLET_GRAPHICS_SYSPIPE_MASK => QueryAnswer::Data {
                at: 0,
                value: geom
                    .syspipe_masks
                    .map_or(u32::from(geom.gfx_gpc_mask != 0), |m| m.graphics_syspipe),
            },
            // ⊘ RM's own answer on this part. Type 5 is refused on EVERY part, deprecated;
            // 7, 8, 9 and 12 each carry an explicit "Does not support … legacy case".
            query_type::PARTITION_CHIPLET_GPC_MAP
            | query_type::PARTITION_CHIPLET_SYSPIPE_IDS
            | query_type::PROFILER_MON_GPC_MASK
            | query_type::PARTITION_SYSPIPE_ID
            | query_type::GFX_CAPABLE_GPC_MASK => QueryAnswer::RefusedByHardware {
                status: NV_ERR_NOT_SUPPORTED,
            },
            // ⊘ Anything else is a type this port does not model ⇒ whole-call refusal, not a
            // per-query one. See this method's doc comment.
            _ => QueryAnswer::Unmodelled,
        }
    }
}

/// Why a `GR_FS_INFO` reply could not be built. Every variant refuses the **whole** control —
/// which is what RM does for a structural fault, and what this port additionally does for a
/// query type it does not model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GrFsInfoError {
    /// The params buffer is shorter than the struct.
    ShortParams {
        /// What arrived.
        len: usize,
        /// [`GR_FS_INFO_PARAMS_SIZE`].
        need: usize,
    },
    /// `numQueries` is zero or above [`MAX_QUERIES`] — the structural fault RM fails the
    /// whole call on (`ogkm-580: ctrl2080grmgr.h:42-50`).
    QueryCount {
        /// What the guest asked for.
        asked: u16,
        /// [`MAX_QUERIES`].
        max: usize,
    },
    /// ⊘ A query type this port does not model. Refused **loudly**, as the whole control,
    /// rather than as an invisible per-query `status` inside an `NV_OK` reply.
    UnmodelledQuery {
        /// The offending `queryType`.
        query_type: u16,
        /// Which element of the batch it was.
        index: usize,
    },
}

impl core::fmt::Display for GrFsInfoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ShortParams { len, need } => {
                write!(f, "{len}-byte params, {need} needed for the struct")
            }
            Self::QueryCount { asked, max } => write!(
                f,
                "numQueries {asked} is not in 1..={max}; RM fails the whole call on a \
                 structural fault of this kind rather than logging it per query"
            ),
            Self::UnmodelledQuery { query_type, index } => write!(
                f,
                "queries[{index}] asks type {query_type}, which this port does not model; \
                 refused as the whole control BY DESIGN — a per-query NV_ERR_NOT_SUPPORTED \
                 would ride inside an NV_OK reply and reach no ledger"
            ),
        }
    }
}

impl core::error::Error for GrFsInfoError {}

/// Answer a `GR_FS_INFO` batch: **the request, edited**.
///
/// Each declared query keeps its `queryType` and its `[IN]` words and gains a `status` and,
/// where applicable, one `[OUT]` word. The tail past `numQueries` is left exactly as it
/// arrived — RM's own loop only ever indexes `0..numQueries`.
///
/// # Errors
///
/// Every variant of [`GrFsInfoError`]. ⊘ Note which faults are *not* here: an out-of-range
/// `gpcId` and a MIG-only query type are **answers**, written into the query's own `status`,
/// because that is what a real GA106 does.
pub fn answer_gr_fs_info(
    request: &[u8],
    geom: &GrFsGeometry<'_>,
) -> Result<Vec<u8>, GrFsInfoError> {
    let Some(body) = request.get(..GR_FS_INFO_PARAMS_SIZE) else {
        return Err(GrFsInfoError::ShortParams {
            len: request.len(),
            need: GR_FS_INFO_PARAMS_SIZE,
        });
    };
    let mut out = body.to_vec();

    let count = u16::from_le_bytes([out[0], out[1]]);
    if count == 0 || count as usize > MAX_QUERIES {
        return Err(GrFsInfoError::QueryCount {
            asked: count,
            max: MAX_QUERIES,
        });
    }

    for i in 0..count as usize {
        // In range by construction: `count <= 96` and the buffer is `8 + 96 * 20` long.
        let at = QUERIES_OFF + QUERY_STRIDE * i;
        let q = GrFsQuery {
            query_type: u16::from_le_bytes([
                out[at + QUERY_TYPE_OFF],
                out[at + QUERY_TYPE_OFF + 1],
            ]),
            input: u32::from_le_bytes([
                out[at + QUERY_DATA_OFF],
                out[at + QUERY_DATA_OFF + 1],
                out[at + QUERY_DATA_OFF + 2],
                out[at + QUERY_DATA_OFF + 3],
            ]),
        };
        let (status, data) = match q.answer(geom) {
            QueryAnswer::Data { at: off, value } => (0u32, Some((off, value))),
            QueryAnswer::RefusedByHardware { status } => (status, None),
            QueryAnswer::Unmodelled => {
                return Err(GrFsInfoError::UnmodelledQuery {
                    query_type: q.query_type,
                    index: i,
                });
            }
        };
        out[at + QUERY_STATUS_OFF..at + QUERY_STATUS_OFF + 4]
            .copy_from_slice(&status.to_le_bytes());
        if let Some((off, value)) = data {
            let w = at + QUERY_DATA_OFF + off;
            out[w..w + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    Ok(out)
}

/// Build a request — for tests and for any future sweep. ⊘ A sweep must use this rather than
/// a `0xCD` seed: `numQueries`, `queryType` and the per-type input words are `[IN]`.
#[must_use]
pub fn build_request(queries: &[GrFsQuery]) -> Vec<u8> {
    let mut p = vec![0u8; GR_FS_INFO_PARAMS_SIZE];
    let n = u16::try_from(queries.len()).unwrap_or(u16::MAX);
    p[0..2].copy_from_slice(&n.to_le_bytes());
    for (i, q) in queries.iter().enumerate().take(MAX_QUERIES) {
        let at = QUERIES_OFF + QUERY_STRIDE * i;
        p[at..at + 2].copy_from_slice(&q.query_type.to_le_bytes());
        p[at + QUERY_DATA_OFF..at + QUERY_DATA_OFF + 4].copy_from_slice(&q.input.to_le_bytes());
    }
    p
}

/// Read back `(status, queryData[0..4] as u32, queryData[4..8] as u32)` per declared query —
/// for tests and the trace differential, so a comparison is on decoded values rather than a
/// hex string somebody regrouped by hand.
///
/// # Errors
///
/// [`GrFsInfoError::ShortParams`] or [`GrFsInfoError::QueryCount`].
pub fn decode_answers(params: &[u8]) -> Result<Vec<(u16, u32, u32, u32)>, GrFsInfoError> {
    let Some(body) = params.get(..GR_FS_INFO_PARAMS_SIZE) else {
        return Err(GrFsInfoError::ShortParams {
            len: params.len(),
            need: GR_FS_INFO_PARAMS_SIZE,
        });
    };
    let count = u16::from_le_bytes([body[0], body[1]]);
    if count == 0 || count as usize > MAX_QUERIES {
        return Err(GrFsInfoError::QueryCount {
            asked: count,
            max: MAX_QUERIES,
        });
    }
    let w = |at: usize| u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]]);
    Ok((0..count as usize)
        .map(|i| {
            let at = QUERIES_OFF + QUERY_STRIDE * i;
            (
                u16::from_le_bytes([body[at], body[at + 1]]),
                w(at + QUERY_STATUS_OFF),
                w(at + QUERY_DATA_OFF),
                w(at + QUERY_DATA_OFF + 4),
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ga106() -> GrFsGeometry<'static> {
        GrFsGeometry::from_profile(&crate::grstatic::GA106_GR_STATIC)
    }

    const fn row(physical_id: u32, tpc_mask: u32) -> GpcRow {
        GpcRow {
            physical_id,
            tpc_mask,
            tpc_count: tpc_mask.count_ones(),
            mmu_per_gpc: 1,
            num_pes_per_gpc: 2,
            zcull_mask: 0xf,
            ppc_mask: Some(0x3),
            rop_mask: Some(0x3),
        }
    }

    /// ★ The RTX 3060 Ti's GR as its host answered it (`[measured 2026-09-26]`, module header):
    /// `gpcMask = 0x3e`, logical GPC `i` is physical `i + 1`, physical GPC 1 has three TPCs.
    static GA104_3060TI: [GpcRow; 5] = [
        row(1, 0xe),
        row(2, 0xf),
        row(3, 0xf),
        row(4, 0xf),
        row(5, 0xf),
    ];

    fn ga104() -> GrFsGeometry<'static> {
        GrFsGeometry {
            gfx_gpc_mask: 0x3e,
            gpcs: &GA104_3060TI,
            syspipe_masks: Some(GrSyspipeMasks {
                syspipe: 1,
                graphics_syspipe: 0,
            }),
        }
    }

    /// ★★★ The pin: libcuda's own three queries, answered as a real GA106 answers them.
    /// `[measured 2026-08-09, cuinit_ioctl_trace_real_ga106.txt:64]` — the only two bytes
    /// that change in the whole 1928 are at offsets 40 and 60, `0x01` and `0x02`.
    #[test]
    fn libcudas_three_queries_reproduce_the_real_reply() {
        let req = build_request(&[
            GrFsQuery {
                query_type: query_type::CHIPLET_GPC_MAP,
                input: 0,
            },
            GrFsQuery {
                query_type: query_type::CHIPLET_GPC_MAP,
                input: 1,
            },
            GrFsQuery {
                query_type: query_type::CHIPLET_GPC_MAP,
                input: 2,
            },
        ]);
        let out = answer_gr_fs_info(&req, &ga106()).expect("served");
        let diff: Vec<usize> = (0..GR_FS_INFO_PARAMS_SIZE)
            .filter(|&k| req[k] != out[k])
            .collect();
        assert_eq!(diff, [40, 60], "exactly the two bytes hardware changes");
        assert_eq!(out[40], 0x01);
        assert_eq!(out[60], 0x02);
        assert_eq!(
            decode_answers(&out).expect("decode"),
            [(2, 0, 0, 0), (2, 0, 1, 1), (2, 0, 2, 2)]
        );
    }

    /// ⊘ The whole tail past `numQueries` is untouched, including a seeded one.
    #[test]
    fn the_tail_past_num_queries_is_left_alone() {
        let mut req = build_request(&[GrFsQuery {
            query_type: query_type::GPC_COUNT,
            input: 0,
        }]);
        for b in req.iter_mut().skip(QUERIES_OFF + QUERY_STRIDE) {
            *b = 0xAA;
        }
        let out = answer_gr_fs_info(&req, &ga106()).expect("served");
        assert_eq!(
            out[QUERIES_OFF + QUERY_STRIDE..],
            req[QUERIES_OFF + QUERY_STRIDE..]
        );
        assert_eq!(decode_answers(&out).expect("decode")[0], (1, 0, 3, 0));
    }

    /// ★★ The three-outcome split, which is the design: a MIG-only type is a per-query
    /// refusal inside an `NV_OK` reply, and an unmodelled type takes the whole call down.
    #[test]
    fn mig_only_types_refuse_per_query_and_unmodelled_types_refuse_the_call() {
        for qt in [
            query_type::PARTITION_CHIPLET_GPC_MAP,
            query_type::PARTITION_CHIPLET_SYSPIPE_IDS,
            query_type::PROFILER_MON_GPC_MASK,
            query_type::PARTITION_SYSPIPE_ID,
            query_type::GFX_CAPABLE_GPC_MASK,
        ] {
            let req = build_request(&[GrFsQuery {
                query_type: qt,
                input: 0,
            }]);
            let out = answer_gr_fs_info(&req, &ga106())
                .unwrap_or_else(|e| panic!("type {qt} must be SERVED with a refused query: {e}"));
            assert_eq!(
                decode_answers(&out).expect("decode")[0].1,
                NV_ERR_NOT_SUPPORTED,
                "type {qt}"
            );
        }
        // ⊘ `TPC_MASK` left this list on 2026-09-26 (modelled: the row's own mask). `PPC_MASK`
        // and `ROP_MASK` stay here for a profile that did not measure them — the fixture.
        for qt in [
            query_type::INVALID,
            query_type::PPC_MASK,
            query_type::ROP_MASK,
            13,
            0xffff,
        ] {
            let req = build_request(&[GrFsQuery {
                query_type: qt,
                input: 0,
            }]);
            assert!(
                matches!(
                    answer_gr_fs_info(&req, &ga106()),
                    Err(GrFsInfoError::UnmodelledQuery { query_type, index: 0 }) if query_type == qt
                ),
                "type {qt} must take the WHOLE call down, not hide in a per-query status"
            );
        }
    }

    /// An out-of-range `gpcId` is the caller's fault and is logged per query — RM marches on.
    /// ⊘ The status was `NV_ERR_NOT_SUPPORTED` until 2026-09-26, chosen without a measurement;
    /// a real GPU writes `NV_ERR_INVALID_ARGUMENT` (module header).
    #[test]
    fn an_out_of_range_gpc_is_a_per_query_fault_not_a_call_fault() {
        let req = build_request(&[
            GrFsQuery {
                query_type: query_type::CHIPLET_GPC_MAP,
                input: 3,
            },
            GrFsQuery {
                query_type: query_type::GPC_COUNT,
                input: 0,
            },
        ]);
        let out = answer_gr_fs_info(&req, &ga106()).expect("served");
        let a = decode_answers(&out).expect("decode");
        assert_eq!(a[0].1, NV_ERR_INVALID_ARGUMENT);
        assert_eq!(a[1], (1, 0, 3, 0), "the batch marched on");
    }

    /// ⊘ A structural fault fails the whole call, which is RM's own rule.
    #[test]
    fn a_bad_query_count_fails_the_whole_call() {
        let mut req = build_request(&[GrFsQuery {
            query_type: query_type::GPC_COUNT,
            input: 0,
        }]);
        req[0..2].copy_from_slice(&0u16.to_le_bytes());
        assert!(matches!(
            answer_gr_fs_info(&req, &ga106()),
            Err(GrFsInfoError::QueryCount { asked: 0, .. })
        ));
        req[0..2].copy_from_slice(&97u16.to_le_bytes());
        assert!(matches!(
            answer_gr_fs_info(&req, &ga106()),
            Err(GrFsInfoError::QueryCount { asked: 97, .. })
        ));
        req[0..2].copy_from_slice(&96u16.to_le_bytes());
        // 96 is legal as a COUNT; the batch then fails on the first unmodelled type, which
        // is a different error and proves the bound is `<=` and not `<`.
        assert!(matches!(
            answer_gr_fs_info(&req, &ga106()),
            Err(GrFsInfoError::UnmodelledQuery { index: 1, .. })
        ));
    }

    /// ★ The logical→physical map is the ROW's, not "the n-th set bit" — checked on a part
    /// where the two differ even though the mask is contiguous (`traces/rpctrace_ga106_boot1.bin`:
    /// an RTX 3060 whose four-TPC GPC is physical 1, so logical 0 → physical 1).
    #[test]
    fn the_gpc_map_is_the_rows_not_the_nth_set_bit() {
        static RPCTRACE_GA106: [GpcRow; 3] = [row(1, 0x1b), row(0, 0x1f), row(2, 0x1f)];
        let g = GrFsGeometry {
            gfx_gpc_mask: 0b111,
            gpcs: &RPCTRACE_GA106,
            syspipe_masks: None,
        };
        assert_eq!(
            g.physical_gpc(0),
            Some(1),
            "the n-th-set-bit rule would say 0"
        );
        assert_eq!(g.physical_gpc(1), Some(0));
        assert_eq!(g.physical_gpc(2), Some(2));
        assert_eq!(g.physical_gpc(3), None);
        // ⊘ And on the fixture board the identity holds — which is why it could not tell.
        for i in 0..3 {
            assert_eq!(ga106().physical_gpc(i), Some(i));
        }
    }

    /// ★★★ The RTX 3060 Ti's host, reproduced: every per-GPC type it answers, with its
    /// logical `gpcId`, its values, and its `0x1f` past the fifth GPC — and the syspipe words.
    #[test]
    fn a_floorswept_hosts_own_answers_are_served() {
        let mut qs = vec![
            GrFsQuery {
                query_type: query_type::GPC_COUNT,
                input: 0,
            },
            GrFsQuery {
                query_type: query_type::CHIPLET_SYSPIPE_MASK,
                input: 0,
            },
            GrFsQuery {
                query_type: query_type::CHIPLET_GRAPHICS_SYSPIPE_MASK,
                input: 0,
            },
        ];
        for t in [
            query_type::CHIPLET_GPC_MAP,
            query_type::TPC_MASK,
            query_type::PPC_MASK,
            query_type::ROP_MASK,
        ] {
            for gpc in 0..6 {
                qs.push(GrFsQuery {
                    query_type: t,
                    input: gpc,
                });
            }
        }
        let out = answer_gr_fs_info(&build_request(&qs), &ga104()).expect("served");
        let a = decode_answers(&out).expect("decode");
        assert_eq!(a[0], (query_type::GPC_COUNT, 0, 5, 0));
        assert_eq!(a[1], (query_type::CHIPLET_SYSPIPE_MASK, 0, 1, 0));
        assert_eq!(
            a[2],
            (query_type::CHIPLET_GRAPHICS_SYSPIPE_MASK, 0, 0, 0),
            "the host's 0"
        );
        let per = |t: usize| &a[3 + 6 * t..3 + 6 * t + 6];
        assert_eq!(
            per(0).iter().map(|q| q.3).collect::<Vec<_>>(),
            [1, 2, 3, 4, 5, 0],
            "CHIPLET_GPC_MAP"
        );
        assert_eq!(
            per(1).iter().map(|q| q.3).collect::<Vec<_>>(),
            [0xe, 0xf, 0xf, 0xf, 0xf, 0],
            "TPC_MASK"
        );
        assert_eq!(
            per(2).iter().map(|q| q.3).collect::<Vec<_>>(),
            [3, 3, 3, 3, 3, 0],
            "PPC_MASK"
        );
        assert_eq!(
            per(3).iter().map(|q| q.3).collect::<Vec<_>>(),
            [3, 3, 3, 3, 3, 0],
            "ROP_MASK"
        );
        for t in 0..4 {
            assert!(
                per(t)[..5].iter().all(|q| q.1 == 0),
                "type {t}: in range is NV_OK"
            );
            assert_eq!(
                per(t)[5].1,
                NV_ERR_INVALID_ARGUMENT,
                "type {t}: gpcId 5 is past the count"
            );
        }
    }

    /// The layout, against the header rather than against itself.
    #[test]
    fn the_struct_layout_is_the_headers() {
        assert_eq!(MAX_QUERIES, 96);
        assert_eq!(QUERY_STRIDE, 20);
        assert_eq!(QUERIES_OFF, 8);
        assert_eq!(GR_FS_INFO_PARAMS_SIZE, 1928);
        // ⊘ NOT `NV2080_CTRL_GRMGR_GR_FS_INFO_QUERY_MAX_SIZE = 32`, which would give 3080.
        assert_ne!(QUERIES_OFF + MAX_QUERIES * 32, GR_FS_INFO_PARAMS_SIZE);
    }

    #[test]
    fn a_short_buffer_is_refused_rather_than_padded() {
        let short = vec![0u8; GR_FS_INFO_PARAMS_SIZE - 1];
        assert!(matches!(
            answer_gr_fs_info(&short, &ga106()),
            Err(GrFsInfoError::ShortParams { need: 1928, .. })
        ));
    }
}
