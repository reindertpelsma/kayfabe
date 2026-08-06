//! The five `GSP_RM_CONTROL` reply bodies that make up **GR's static configuration** —
//! `KGR_GET_CAPS` (`0x20800a1f`), `..._FLOORSWEEPING_MASKS` (`0x20800a26`),
//! `..._GLOBAL_SM_ORDER` (`0x20800a22`), `..._FECS_RECORD_SIZE` (`0x20800a3d`) and
//! `..._PDB_PROPERTIES` (`0x20800a48`) — and ★★★ **the first thing this port says about the
//! shape of the shader core rather than about a bus, a table, or an identity.**
//!
//! # ★★★ Why these five, and not the fourteen GR asks for
//!
//! `kgraphicsLoadStaticInfo_KERNEL` (`ogkm-580:
//! src/nvidia/src/kernel/gpu/gr/kernel_graphics.c:1121-1543`) issues **fourteen** controls in
//! one straight line. They are not equal, and the difference is visible in the source at each
//! call site rather than in the control's name:
//!
//! | control | id | how the status is handled | mandatory? |
//! |---|---|---|---|
//! | `KGR_GET_CAPS` | `0x20800a1f` | `NV_CHECK_OK_OR_GOTO(cleanup)` :1212 | ★ **yes** |
//! | `..._GET_INFO` | `0x20800a2a` | `if (status == NV_OK) { … }`, no else :1234 | no |
//! | `..._FLOORSWEEPING_MASKS` | `0x20800a26` | `NV_CHECK_OK_OR_GOTO(cleanup)` :1252 | ★ **yes** |
//! | `..._GLOBAL_SM_ORDER` | `0x20800a22` | `NV_CHECK_OK_OR_GOTO(cleanup)` :1284 | ★ **yes** |
//! | `..._PPC_MASKS` | `0x20800a30` | explicit `NOT_SUPPORTED → NV_OK` :1320 | no |
//! | `..._ZCULL_INFO` | `0x20800a2c` | `NOT_SUPPORTED → NV_OK` **only under MIG** :1349 | ⚠ see below |
//! | `..._ROP_INFO` | `0x20800a2e` | `NOT_SUPPORTED → NV_OK` **only under MIG** :1379 | ⚠ see below |
//! | `..._SM_ISSUE_RATE_MODIFIER` | — | explicit `NOT_SUPPORTED → NV_OK` :1410 | no |
//! | `..._SM_ISSUE_RATE_MODIFIER_V2` | — | explicit `NOT_SUPPORTED → NV_OK` :1435 | no |
//! | `..._SM_ISSUE_THROTTLE_CTRL` | — | explicit `NOT_SUPPORTED → NV_OK` :1461 | no |
//! | `..._FECS_RECORD_SIZE` | `0x20800a3d` | `NV_CHECK_OK_OR_GOTO(cleanup)` :1467 | ★ **yes** |
//! | `..._FECS_TRACE_DEFINES` | `0x20800a3f` | explicit `NOT_SUPPORTED → NV_OK` :1498 | no |
//! | `..._PDB_PROPERTIES` | `0x20800a48` | `NV_CHECK_OK_OR_GOTO(cleanup)` :1506 | ★ **yes** |
//! | `..._CONTEXT_BUFFERS_INFO` | `0x20800a32` | `NV_CHECK_OK_OR_GOTO(cleanup)` :1527 | ⚠ **conditional** |
//!
//! ## ⚠ ZCULL and ROP look mandatory and are not — the status is CLOBBERED
//!
//! Their `else if` arms convert `NV_ERR_NOT_SUPPORTED` to `NV_OK` **only when MIG is
//! enabled**, and MIG is not enabled on this chip. Read alone, each therefore leaves
//! `status = 0x56`. But neither arm branches: control falls into the next paragraph, whose
//! first act is `status = pRmApi->Control(…)` — an **assignment**, not a comparison. ZCULL's
//! `0x56` is overwritten by ROP's call at `:1360`, and ROP's by
//! `SM_ISSUE_RATE_MODIFIER`'s at `:1391`, which *does* normalise `0x56` to `NV_OK`
//! unconditionally. ⇒ both are survivable, for a reason that is not in either `else if`.
//!
//! ⊘ This correction is recorded because getting it wrong costs a boot in the expensive
//! direction: a reading that called them mandatory would have had this port encode two more
//! bodies — 320 and 96 bytes of ZCULL geometry and ROP counts — that nothing reads, and it
//! would have called that necessary. **Two fewer statements about silicon we have not
//! modelled is the better outcome**, and `SWEEP_TRIAGE` carries them as deliberate refusals.
//!
//! ## ⚠ `CONTEXT_BUFFERS_INFO` is behind a condition this port has not evaluated
//!
//! `:1524-1529` runs `kgraphicsInitializeDeferredStaticData` — which issues `0x20800a32` —
//! only `if (IS_MIG_IN_USE(pGpu) || !kgraphicsShouldDeferContextInit(pGpu, pKernelGraphics))`.
//! MIG is off, so it turns entirely on `kgraphicsShouldDeferContextInit`. ⊘ **`[assumed]`,
//! and named as such**: this port has not established which way that predicate goes on
//! GA106, and it is deliberately *not* being guessed at, because the boot settles it for
//! free. `bInitialized = NV_TRUE` is set at `:1521`, **before** this branch; if the branch is
//! taken and `0x20800a32` is refused, `cleanup:` sets it back to `NV_FALSE` (`:1544`) and the
//! rung fails in exactly the way it failed before, with `0x20800a32` newly visible in the
//! device's unserviced list. That is a *distinguishable* outcome, which is why guessing is
//! unnecessary. See `docs/design/boot_measured_2026_08_01.md`.
//!
//! # ★★★ Why the fatal error is nowhere near these controls
//!
//! Refusing `0x20800a1f` does not stop `gpuStatePostLoad` — `gpu.c:3438` maps
//! `NV_ERR_NOT_SUPPORTED` to `NV_OK`, exactly like the pre-init sweep
//! (`crate::super`… see `kayfabe_device::sweep`). What it does is send
//! `kgraphicsLoadStaticInfo_KERNEL` to its `cleanup:` label, which sets
//! `pPrivate->bInitialized = NV_FALSE` (`:1544`), which makes `kgraphicsGetStaticInfo_IMPL`
//! return **`NULL` forever** (`:556-564`).
//!
//! The bill arrives twenty-one engines later. `kgraphicsStateLoad_IMPL` registered
//! `_kgraphicsPostSchedulingEnableHandler` as a FIFO callback (`:373-377`); `KernelFifo`'s
//! own `statePostLoad` runs it (`kfifoStatePostLoad_GM107` → `kernel_fifo.c:3111`); and its
//! fourth statement is
//!
//! ```c
//! NV_ASSERT_OR_RETURN(pKernelGraphicsStaticInfo != NULL, NV_ERR_INVALID_STATE);
//! ```
//!
//! (`kernel_graphics.c:485`) — `NV_ERR_INVALID_STATE`, `0x40`, which `gpu.c:3440` does **not**
//! swallow because it is not `0x56`. ⇒ `RmInitAdapter failed! (0x25:0x40:1249)`.
//!
//! ★★ So this is the sweep's signature failure at its purest: **the refusal is silent, the
//! subsystem that dies is not the one that asked, and the distance between them is the whole
//! engine order.** `[measured]` run `gmmu1`, a stock 580.159.04 guest at `12b001f` — the
//! dmesg carries `kgraphicsLoadStaticInfo` at `LEVEL_ERROR` and the `0x40` twenty lines later
//! with nothing connecting them.
//!
//! # ★★★ The rejected shortcut, named so it stays rejected
//!
//! `_kgraphicsPostSchedulingEnableHandler` returns `NV_OK` immediately when
//! `floorsweepingMasks.gpcMask == 0x0` (`kernel_graphics.c:486-487`). ⇒ **answering
//! `gpcMask = 0` would carry the boot straight past `gpuStatePostLoad`** — no golden-image
//! channel, no `0x40`, a longer green log, and one fewer wall tonight.
//!
//! ⊘ **It is a lie about the chip and this port does not tell it.** No GA106 has zero GPCs;
//! `crate::chipinfo` already publishes an identity that says otherwise; and the value would
//! then be read by every later GR decision as though it had been modelled. It is the
//! flattering-instrument shape — a *served* answer whose only merit is that it makes the
//! next check not run — and it is exactly what `RefusalFailsOpen` exists to forbid. The
//! honest value is `0x7`, and the honest consequence is that the boot proceeds into
//! `kgraphicsCreateGoldenImageChannel` (`:508`) and stops there instead, on machinery this
//! port has not built. That is the wall we want: further on, and about something real.
//!
//! # ★★ Where every number below comes from
//!
//! `[measured]`, and the run is the **C artifact's rather than this port's**. The oracle's
//! captured init-control table is *"GA106 init `GSP_RM_CONTROL` responses (real, captured
//! from host)"* (`C: src/qemu/mode2_initctrl_ga106.h:1-2`, `nvidia-gpu-passthrough` rev
//! `8baf4f2`), and it carries all five with `status = 0x0`:
//!
//! ```text
//! {0x20800a1fu, 0x0u,   184u,   176u, ctl_20800a1f},   /* caps          */
//! {0x20800a26u, 0x0u,  3008u,  3008u, ctl_20800a26},   /* floorsweeping */
//! {0x20800a22u, 0x0u, 34592u, 16376u, ctl_20800a22},   /* SM order      */
//! {0x20800a3du, 0x0u,    32u,    32u, ctl_20800a3d},   /* FECS rec size */
//! {0x20800a48u, 0x0u,     8u,     8u, ctl_20800a48},   /* PDB props     */
//! ```
//!
//! ⚠ **`psize` is the whole eight-engine array, and `dlen` is what the recorder kept.** The
//! caller passes `sizeof(pParams->caps)` — the union member, all eight engines — as
//! `paramsSize` (`kernel_graphics.c:1219`), so the reply this port must produce is
//! `params_size()` bytes even though only `engineCaps[0]` is ever read
//! (`grIdx = 0`, single GR engine, no MIG). The other seven are zero in the capture and zero
//! here, and `tests/gr_static_info.rs` is what says so rather than a comment.
//!
//! ⊘ **`0x20800a22`'s capture is TRUNCATED and the differential knows it.** `dlen` 16 376 of
//! `psize` 34 592: the recorder kept one message-queue element's worth. Engine 0 occupies the
//! first 4 324 bytes, so nothing that is read is missing — but the fixture can only pin the
//! prefix, and `tests/gr_static_info.rs` compares exactly `dlen` bytes and says why. Beyond
//! it the encoder is checked by structure (`params_size()`) and by the all-zero property,
//! not against the oracle.
//!
//! # ★★ What this module is NOT
//!
//! It is not a replay. Every field below is **named**, given a type, and re-derived — the
//! floorsweeping masks from three GPC rows, the 28 SM entries from fourteen TPC rows and a
//! constant two SMs each. That is what makes the fixture comparison a *test* rather than a
//! tautology: it can fail on a wrong offset, a wrong stride, a wrong array bound, or a
//! wrong endianness, and those are precisely the mistakes that would otherwise produce a
//! plausible reply the guest silently believes.
//!
//! ⊘ And it is not a claim that these numbers describe **the host GPU of whoever runs this**.
//! They describe a GA106 — the part this device *presents* (`crate::chipinfo`,
//! `kayfabe_device::ga10x`). The day this port forwards to a real host GPU whose GR differs,
//! this table is a lie of a different kind, and `docs/design/mode2_forwarding_model.md` is
//! where that has to be re-decided rather than inherited.

// ---------------------------------------------------------------------------------------
// Control ids
// ---------------------------------------------------------------------------------------

/// `NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_CAPS`
/// (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080internal.h:211`).
pub const NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_CAPS: u32 = 0x2080_0a1f;
/// `NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_FLOORSWEEPING_MASKS` (`ctrl2080internal.h:334`).
pub const NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_FLOORSWEEPING_MASKS: u32 = 0x2080_0a26;
/// `NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_GLOBAL_SM_ORDER` (`ctrl2080internal.h:249`).
pub const NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_GLOBAL_SM_ORDER: u32 = 0x2080_0a22;
/// `NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_FECS_RECORD_SIZE` (`ctrl2080internal.h:683`).
pub const NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_FECS_RECORD_SIZE: u32 = 0x2080_0a3d;
/// `NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_PDB_PROPERTIES` (`ctrl2080internal.h:866`).
pub const NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_PDB_PROPERTIES: u32 = 0x2080_0a48;

// ---------------------------------------------------------------------------------------
// Shapes, all from ogkm-580 rather than from any capture
// ---------------------------------------------------------------------------------------

/// `NV2080_CTRL_INTERNAL_GR_MAX_ENGINES` (`ctrl2080internal.h:192`) — the outer array bound
/// on **every** one of these five params structs. Only index `grIdx` is read, and `grIdx` is
/// `0` for a single-GR, non-MIG chip (`kernel_graphics.c:1190-1208`).
pub const GR_MAX_ENGINES: usize = 8;
/// `NV2080_CTRL_INTERNAL_GR_MAX_GPC` (`ctrl2080internal.h:286`).
pub const GR_MAX_GPC: usize = 16;
/// `NV2080_CTRL_INTERNAL_MAX_TPC_PER_GPC_COUNT` (`ctrl2080internal.h:287`).
pub const MAX_TPC_PER_GPC: usize = 10;
/// `NV2080_CTRL_INTERNAL_GR_MAX_SM` (`ctrl2080internal.h:243`).
pub const GR_MAX_SM: usize = 240;
/// `NV0080_CTRL_GR_CAPS_TBL_SIZE` (`ogkm-580: ctrl/ctrl0080/ctrl0080gr.h:78`).
pub const GR_CAPS_TBL_SIZE: usize = 23;

/// `sizeof(NV2080_CTRL_INTERNAL_STATIC_GR_GET_CAPS_PARAMS)` — `8 * 23`.
pub const GR_CAPS_PARAMS_SIZE: usize = GR_MAX_ENGINES * GR_CAPS_TBL_SIZE;
/// `sizeof(NV2080_CTRL_INTERNAL_STATIC_GR_FLOORSWEEPING_MASKS)` — 94 `NvU32`s; see
/// [`encode_floorsweeping_masks`] for the field-by-field derivation of that count.
pub const FLOORSWEEPING_ROW_SIZE: usize = 4
    * (1 + GR_MAX_GPC
        + GR_MAX_GPC
        + 1
        + GR_MAX_GPC
        + MAX_TPC_PER_GPC
        + GR_MAX_GPC
        + GR_MAX_GPC
        + 1
        + 1);
/// `sizeof(NV2080_CTRL_INTERNAL_STATIC_GR_GET_FLOORSWEEPING_MASKS_PARAMS)` — `8 * 376`.
pub const FLOORSWEEPING_PARAMS_SIZE: usize = GR_MAX_ENGINES * FLOORSWEEPING_ROW_SIZE;
/// One `globalSmId[]` entry: nine `NvU16`s (`ctrl2080internal.h:245-256`).
pub const SM_ENTRY_SIZE: usize = 9 * 2;
/// `sizeof(NV2080_CTRL_INTERNAL_STATIC_GR_GLOBAL_SM_ORDER)` — `240 * 18` plus `numSm` and
/// `numTpc`.
pub const SM_ORDER_ROW_SIZE: usize = GR_MAX_SM * SM_ENTRY_SIZE + 4;
/// `sizeof(NV2080_CTRL_INTERNAL_STATIC_GR_GET_GLOBAL_SM_ORDER_PARAMS)` — `8 * 4324`.
pub const SM_ORDER_PARAMS_SIZE: usize = GR_MAX_ENGINES * SM_ORDER_ROW_SIZE;
/// `sizeof(NV2080_CTRL_INTERNAL_STATIC_GR_GET_FECS_RECORD_SIZE_PARAMS)` — one `NvU32` each.
pub const FECS_RECORD_SIZE_PARAMS_SIZE: usize = GR_MAX_ENGINES * 4;
/// `sizeof(NV2080_CTRL_INTERNAL_STATIC_GR_GET_PDB_PROPERTIES_PARAMS)` — one `NvBool` each.
pub const PDB_PROPERTIES_PARAMS_SIZE: usize = GR_MAX_ENGINES;

// ---------------------------------------------------------------------------------------
// The GA106 rows
// ---------------------------------------------------------------------------------------

/// One GPC's floorsweeping row.
///
/// ⚠ `tpc_mask` is a **physical** bitmap and `tpc_count` is a *count*, and on GA106 they
/// disagree in a way that is not an error: GPC 0 reports `0x1e` (physical TPCs 1-4) with
/// count 4, so the physical bit positions are not `0..count`. Anything that derives a
/// local TPC index from `tpc_mask` by counting from bit 0 is wrong; the header says so
/// itself (`ctrl2080internal.h:298-306`: `tpcMask` is *"indexed by physical GPC ID for
/// non-MIG"*, `tpcCount` *"always indexed by logical GPC ID"*).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpcRow {
    /// Physical TPC bitmap for this GPC.
    pub tpc_mask: u32,
    /// Number of TPCs enabled in this GPC.
    pub tpc_count: u32,
    /// `mmuPerGpc[]` — GMMU instances behind this GPC.
    pub mmu_per_gpc: u32,
    /// `numPesPerGpc[]`.
    pub num_pes_per_gpc: u32,
    /// `zcullMask[]`, indexed by **physical** GPC id.
    pub zcull_mask: u32,
}

/// One TPC of the global SM order, in `globalTpcId` order.
///
/// Each row expands into [`SMS_PER_TPC`] consecutive `globalSmId[]` entries differing only
/// in `localSmId`. That pairing is a **structural claim about the capture** and is checked:
/// `tests/gr_static_info.rs` re-derives all 28 entries from these 14 rows and compares them
/// to the oracle's bytes, so a chip whose SMs were not paired this way could not be
/// described here without the test going red.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TpcRow {
    /// `gpcId` — the physical GPC this TPC belongs to.
    pub gpc_id: u16,
    /// `localTpcId` — this TPC's index **within its GPC**, compacted (not the `tpc_mask`
    /// bit position; see [`GpcRow`]).
    pub local_tpc_id: u16,
    /// `virtualTpcId`.
    pub virtual_tpc_id: u16,
}

/// SMs per TPC on Ampere consumer parts. Two, and the capture's 28 SMs over 14 TPCs is what
/// says so — `[measured]` on a real GA106 (`C: mode2_initctrl_ga106.h:6221` = `0x20800a22`),
/// and re-derived by `tests/gr_static_info.rs`, which fails if the pairing stops holding.
pub const SMS_PER_TPC: u16 = 2;

/// GA106's three GPCs. `[measured]` from `C: mode2_initctrl_ga106.h`'s `ctl_20800a26`.
pub const GA106_GPCS: [GpcRow; 3] = [
    GpcRow {
        tpc_mask: 0x1e,
        tpc_count: 4,
        mmu_per_gpc: 1,
        num_pes_per_gpc: 3,
        zcull_mask: 0xf,
    },
    GpcRow {
        tpc_mask: 0x1f,
        tpc_count: 5,
        mmu_per_gpc: 1,
        num_pes_per_gpc: 3,
        zcull_mask: 0xf,
    },
    GpcRow {
        tpc_mask: 0x1f,
        tpc_count: 5,
        mmu_per_gpc: 1,
        num_pes_per_gpc: 3,
        zcull_mask: 0xf,
    },
];

/// `gpcMask` / `physGpcMask` / `physGfxGpcMask` — all three are `0x7` on this part, and all
/// three are written from this one constant so they cannot drift apart.
pub const GA106_GPC_MASK: u32 = 0b111;

/// `tpcToPesMap[10]` — which PES each local TPC hangs off. `[measured]` on a real GA106
/// (`C: src/qemu/mode2_initctrl_ga106.h:6220 = 0x20800a26`), pinned by `tests/gr_static_info.rs`; the
/// tail past `tpc_count` is zero in the capture and zero here.
pub const GA106_TPC_TO_PES_MAP: [u32; MAX_TPC_PER_GPC] = [0, 0, 1, 1, 2, 2, 0, 0, 0, 0];

/// GA106's fourteen TPCs in `globalTpcId` order — the index **is** the `globalTpcId`.
///
/// ⚠ The order is not sorted by GPC and it is not sorted by local TPC; it is NVIDIA's own
/// interleave, and it is data rather than a rule this port derives. `[measured]` from
/// `ctl_20800a22`'s first 4 324 bytes.
pub const GA106_TPCS: [TpcRow; 14] = [
    TpcRow {
        gpc_id: 1,
        local_tpc_id: 0,
        virtual_tpc_id: 0,
    },
    TpcRow {
        gpc_id: 2,
        local_tpc_id: 0,
        virtual_tpc_id: 0,
    },
    TpcRow {
        gpc_id: 0,
        local_tpc_id: 1,
        virtual_tpc_id: 1,
    },
    TpcRow {
        gpc_id: 1,
        local_tpc_id: 2,
        virtual_tpc_id: 1,
    },
    TpcRow {
        gpc_id: 2,
        local_tpc_id: 2,
        virtual_tpc_id: 1,
    },
    TpcRow {
        gpc_id: 0,
        local_tpc_id: 0,
        virtual_tpc_id: 0,
    },
    TpcRow {
        gpc_id: 1,
        local_tpc_id: 4,
        virtual_tpc_id: 2,
    },
    TpcRow {
        gpc_id: 2,
        local_tpc_id: 4,
        virtual_tpc_id: 2,
    },
    TpcRow {
        gpc_id: 0,
        local_tpc_id: 3,
        virtual_tpc_id: 2,
    },
    TpcRow {
        gpc_id: 1,
        local_tpc_id: 1,
        virtual_tpc_id: 3,
    },
    TpcRow {
        gpc_id: 2,
        local_tpc_id: 1,
        virtual_tpc_id: 3,
    },
    TpcRow {
        gpc_id: 0,
        local_tpc_id: 2,
        virtual_tpc_id: 3,
    },
    TpcRow {
        gpc_id: 1,
        local_tpc_id: 3,
        virtual_tpc_id: 4,
    },
    TpcRow {
        gpc_id: 2,
        local_tpc_id: 3,
        virtual_tpc_id: 4,
    },
];

/// `engineCaps[0].capsTbl` — 23 opaque bytes.
///
/// ⊘ **Opaque, and deliberately not decomposed.** `NV0080_CTRL_GR_CAPS_TBL_SIZE` is a
/// bitfield table whose bits are named by `NV0080_CTRL_GR_CAPS_*` macros that this port has
/// no consumer for; naming twenty of them here would be transcription dressed as modelling.
/// It is carried as bytes, `[measured]` on a real GA106 from `ctl_20800a1f`
/// (`C: src/qemu/mode2_initctrl_ga106.h:6218` = `0x20800a1f`), and `tests/gr_static_info.rs` is the test
/// that fails if a byte of it changes.
pub const GA106_GR_CAPS: [u8; GR_CAPS_TBL_SIZE] = [
    0xb0, 0x62, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x10, 0x01, 0x08, 0x00, 0x10,
    0x10, 0x00, 0x00, 0x00, 0x04, 0xc0, 0x05,
];

/// `fecsRecordSize` — 128 bytes per FECS trace record on this part. `[measured]` on a real
/// GA106 (`C: src/qemu/mode2_initctrl_ga106.h:6228` = `0x20800a3d`), pinned by
/// `tests/gr_static_info.rs`. ⚠ The line was `:6246` until 2026-08-02; that is
/// `0x20800a38`'s row, not this control's.
pub const GA106_FECS_RECORD_SIZE: u32 = 128;

/// `bPerSubCtxheaderSupported` — Ampere supports the per-subcontext context header.
/// `[measured]` on a real GA106 (`ctl_20800a48` is `01 00 00 00 00 00 00 00`,
/// `C: src/qemu/mode2_initctrl_ga106.h:6230` = `0x20800a48`, pinned by
/// `tests/gr_static_info.rs`), and it is consumed
/// immediately: `kgraphicsSetPerSubcontextContextHeaderSupported` (`kernel_graphics.c:1518`).
pub const GA106_PER_SUBCTX_HEADER_SUPPORTED: bool = true;

/// Why a GR static-info body could not be built.
///
/// ⊘ Every variant is a statement this port refuses to make about a chip, not an internal
/// error. Encoding is infallible for the constants above; these exist so that a **future**
/// chip row — or a forwarded host GPU's real geometry — cannot produce a body that RM would
/// read as a working GPU when it is not one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrStaticError {
    /// More GPCs than `NV2080_CTRL_INTERNAL_GR_MAX_GPC`, or none at all. A zero `gpcMask`
    /// is refused **by name** because it is the value that makes
    /// `_kgraphicsPostSchedulingEnableHandler` return early (`kernel_graphics.c:486`) — the
    /// rejected shortcut in this module's header. It must be impossible to reach it by
    /// accident.
    GpcCountOutOfRange {
        /// How many GPC rows the profile carries.
        count: usize,
    },
    /// More TPCs than the SM order can describe, or none.
    TpcCountOutOfRange {
        /// How many TPC rows the profile carries.
        count: usize,
    },
    /// `tpc_count` does not equal `tpc_mask.count_ones()` for some GPC. The two are
    /// independent fields of the same reply and RM uses both; a body in which they disagree
    /// describes no silicon.
    TpcMaskCountMismatch {
        /// Which GPC row disagrees with itself.
        gpc: usize,
        /// Its `tpcMask`.
        mask: u32,
        /// Its `tpcCount`.
        count: u32,
    },
    /// The per-GPC TPC rows do not add up to the number of `GA106_TPCS` entries.
    TpcRowsDoNotMatchGpcCounts {
        /// Sum of every GPC row's `tpc_count`.
        from_gpcs: u32,
        /// Number of TPC rows supplied.
        rows: usize,
    },
    /// `numSm` would exceed `NV2080_CTRL_INTERNAL_GR_MAX_SM`.
    SmCountOutOfRange {
        /// `tpcs.len() * sms_per_tpc`.
        count: usize,
    },
    /// A row names a GPC that does not exist.
    GpcIdOutOfRange {
        /// Index of the offending TPC row.
        row: usize,
        /// The GPC id it names.
        gpc_id: u16,
    },
    /// A context buffer that is PRESENT declares a zero alignment. `memdescAlloc` is
    /// handed this value directly; zero is not a weak alignment, it is an invalid one.
    /// ⊘ Deliberately not checked for `CONTEXT_BUFFER_ABSENT` rows: a real GA106 reports
    /// `alignment == NV_U32_MAX` alongside `size == NV_U32_MAX` for the seven engines it
    /// does not have, and demanding a sane alignment there would refuse real hardware.
    ContextBufferAlignmentZero {
        /// `NV0080_CTRL_FIFO_GET_ENGINE_CONTEXT_PROPERTIES_ENGINE_ID_*` of the row.
        id: usize,
    },
    /// The FECS record size is zero — `fecsBufferMap` divides by it
    /// (`ogkm-580: fecs_event_list.c`), so zero is a divide, not a small buffer.
    FecsRecordSizeZero,
}

/// Everything this port says about one chip's GR, in one value.
///
/// ★ One struct rather than five loose encoders: the five replies are five *views* of one
/// geometry, and RM cross-checks them (it reads `numTpc` out of the SM order and
/// `tpcCount` out of the floorsweeping masks and expects them to agree). Making them
/// separate inputs would make disagreement expressible.
#[derive(Debug, Clone, Copy)]
pub struct GrStaticProfile {
    /// The GPC rows; `gpcMask` is derived from the length.
    pub gpcs: &'static [GpcRow],
    /// The TPC rows in `globalTpcId` order.
    pub tpcs: &'static [TpcRow],
    /// SMs per TPC.
    pub sms_per_tpc: u16,
    /// `tpcToPesMap[]`.
    pub tpc_to_pes_map: [u32; MAX_TPC_PER_GPC],
    /// The opaque caps table.
    pub caps: [u8; GR_CAPS_TBL_SIZE],
    /// `fecsRecordSize`.
    pub fecs_record_size: u32,
    /// `bPerSubCtxheaderSupported`.
    pub per_subctx_header_supported: bool,
}

/// GA106 — the part this device presents.
pub const GA106_GR_STATIC: GrStaticProfile = GrStaticProfile {
    gpcs: &GA106_GPCS,
    tpcs: &GA106_TPCS,
    sms_per_tpc: SMS_PER_TPC,
    tpc_to_pes_map: GA106_TPC_TO_PES_MAP,
    caps: GA106_GR_CAPS,
    fecs_record_size: GA106_FECS_RECORD_SIZE,
    per_subctx_header_supported: GA106_PER_SUBCTX_HEADER_SUPPORTED,
};

impl GrStaticProfile {
    /// The `gpcMask` this profile publishes — and the same value for `physGpcMask` and
    /// `physGfxGpcMask`, which is why there is one accessor and not three fields.
    ///
    /// # Errors
    /// [`GrStaticError::GpcCountOutOfRange`] for zero GPCs or more than [`GR_MAX_GPC`].
    pub fn gpc_mask(&self) -> Result<u32, GrStaticError> {
        let n = self.gpcs.len();
        if n == 0 || n > GR_MAX_GPC {
            return Err(GrStaticError::GpcCountOutOfRange { count: n });
        }
        // `n <= 16`, so the shift cannot overflow and the mask cannot be zero.
        Ok((1u32 << n) - 1)
    }

    /// Total SMs — `tpcs.len() * sms_per_tpc`.
    ///
    /// # Errors
    /// [`GrStaticError::SmCountOutOfRange`] if it would not fit `globalSmId[]`.
    pub fn num_sm(&self) -> Result<u16, GrStaticError> {
        let n = self.tpcs.len() * self.sms_per_tpc as usize;
        if n == 0 || n > GR_MAX_SM {
            return Err(GrStaticError::SmCountOutOfRange { count: n });
        }
        u16::try_from(n).map_err(|_| GrStaticError::SmCountOutOfRange { count: n })
    }

    /// ★★ The consistency check every encoder runs first, so that no single reply can be
    /// built out of a geometry the *other* replies would contradict.
    ///
    /// # Errors
    /// The geometry variants of [`GrStaticError`].
    pub fn validate(&self) -> Result<(), GrStaticError> {
        self.gpc_mask()?;
        self.num_sm()?;
        if self.tpcs.is_empty() || self.tpcs.len() > GR_MAX_SM {
            return Err(GrStaticError::TpcCountOutOfRange {
                count: self.tpcs.len(),
            });
        }
        if self.fecs_record_size == 0 {
            return Err(GrStaticError::FecsRecordSizeZero);
        }
        let mut from_gpcs: u32 = 0;
        for (i, g) in self.gpcs.iter().enumerate() {
            if g.tpc_mask.count_ones() != g.tpc_count {
                return Err(GrStaticError::TpcMaskCountMismatch {
                    gpc: i,
                    mask: g.tpc_mask,
                    count: g.tpc_count,
                });
            }
            from_gpcs = from_gpcs.saturating_add(g.tpc_count);
        }
        if from_gpcs as usize != self.tpcs.len() {
            return Err(GrStaticError::TpcRowsDoNotMatchGpcCounts {
                from_gpcs,
                rows: self.tpcs.len(),
            });
        }
        for (i, t) in self.tpcs.iter().enumerate() {
            if t.gpc_id as usize >= self.gpcs.len() {
                return Err(GrStaticError::GpcIdOutOfRange {
                    row: i,
                    gpc_id: t.gpc_id,
                });
            }
        }
        Ok(())
    }
}

fn put32(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}
fn put16(buf: &mut [u8], off: usize, v: u16) {
    buf[off..off + 2].copy_from_slice(&v.to_le_bytes());
}

/// `NV2080_CTRL_INTERNAL_STATIC_GR_GET_CAPS_PARAMS` — `engineCaps[0]` from the profile,
/// the other seven zero.
///
/// # Errors
/// [`GrStaticError`] if the profile is not self-consistent.
pub fn encode_gr_caps(p: &GrStaticProfile) -> Result<Vec<u8>, GrStaticError> {
    p.validate()?;
    let mut out = vec![0u8; GR_CAPS_PARAMS_SIZE];
    out[..GR_CAPS_TBL_SIZE].copy_from_slice(&p.caps);
    Ok(out)
}

/// `NV2080_CTRL_INTERNAL_STATIC_GR_GET_FLOORSWEEPING_MASKS_PARAMS`.
///
/// The field order is `ctrl2080internal.h:297-332` and is written out longhand below rather
/// than with a running cursor: an off-by-one in a cursor is the defect that produces a
/// *plausible* reply, and the explicit offsets are what the fixture test can catch.
///
/// # Errors
/// [`GrStaticError`] if the profile is not self-consistent.
pub fn encode_floorsweeping_masks(p: &GrStaticProfile) -> Result<Vec<u8>, GrStaticError> {
    p.validate()?;
    let mask = p.gpc_mask()?;
    let mut out = vec![0u8; FLOORSWEEPING_PARAMS_SIZE];

    // Offsets within one 376-byte row, in declaration order.
    const O_GPC_MASK: usize = 0;
    const O_TPC_MASK: usize = O_GPC_MASK + 4;
    const O_TPC_COUNT: usize = O_TPC_MASK + 4 * GR_MAX_GPC;
    const O_PHYS_GPC_MASK: usize = O_TPC_COUNT + 4 * GR_MAX_GPC;
    const O_MMU_PER_GPC: usize = O_PHYS_GPC_MASK + 4;
    const O_TPC_TO_PES: usize = O_MMU_PER_GPC + 4 * GR_MAX_GPC;
    const O_NUM_PES: usize = O_TPC_TO_PES + 4 * MAX_TPC_PER_GPC;
    const O_ZCULL_MASK: usize = O_NUM_PES + 4 * GR_MAX_GPC;
    const O_PHYS_GFX_GPC_MASK: usize = O_ZCULL_MASK + 4 * GR_MAX_GPC;
    const O_NUM_GFX_TPC: usize = O_PHYS_GFX_GPC_MASK + 4;
    // ★ The size constant and the field walk must agree, and this is where they are made to.
    const _: () = assert!(O_NUM_GFX_TPC + 4 == FLOORSWEEPING_ROW_SIZE);

    let row = &mut out[..FLOORSWEEPING_ROW_SIZE];
    put32(row, O_GPC_MASK, mask);
    put32(row, O_PHYS_GPC_MASK, mask);
    put32(row, O_PHYS_GFX_GPC_MASK, mask);
    let mut gfx_tpc: u32 = 0;
    for (i, g) in p.gpcs.iter().enumerate() {
        put32(row, O_TPC_MASK + 4 * i, g.tpc_mask);
        put32(row, O_TPC_COUNT + 4 * i, g.tpc_count);
        put32(row, O_MMU_PER_GPC + 4 * i, g.mmu_per_gpc);
        put32(row, O_NUM_PES + 4 * i, g.num_pes_per_gpc);
        put32(row, O_ZCULL_MASK + 4 * i, g.zcull_mask);
        gfx_tpc = gfx_tpc.saturating_add(g.tpc_count);
    }
    for (i, v) in p.tpc_to_pes_map.iter().enumerate() {
        put32(row, O_TPC_TO_PES + 4 * i, *v);
    }
    // ⊘ `numGfxTpc` is the SUM, not `tpcs.len()`. They are equal here and `validate()` is
    // what makes them equal; deriving it from the same place twice would hide a disagreement
    // rather than refuse it.
    put32(row, O_NUM_GFX_TPC, gfx_tpc);
    Ok(out)
}

/// `NV2080_CTRL_INTERNAL_STATIC_GR_GET_GLOBAL_SM_ORDER_PARAMS`.
///
/// Expands each [`TpcRow`] into [`GrStaticProfile::sms_per_tpc`] consecutive entries whose
/// only differing field is `localSmId`. `globalTpcId` is the row index; `virtualGpcId`,
/// `migratableTpcId`, `ugpuId` and `physicalCpcId` are zero on this part.
///
/// # Errors
/// [`GrStaticError`] if the profile is not self-consistent.
pub fn encode_global_sm_order(p: &GrStaticProfile) -> Result<Vec<u8>, GrStaticError> {
    p.validate()?;
    let num_sm = p.num_sm()?;
    let num_tpc = u16::try_from(p.tpcs.len()).map_err(|_| GrStaticError::TpcCountOutOfRange {
        count: p.tpcs.len(),
    })?;
    let mut out = vec![0u8; SM_ORDER_PARAMS_SIZE];

    // Field order within one 18-byte entry (`ctrl2080internal.h:246-256`).
    const O_GPC: usize = 0;
    const O_LOCAL_TPC: usize = 2;
    const O_LOCAL_SM: usize = 4;
    const O_GLOBAL_TPC: usize = 6;
    const O_VIRTUAL_GPC: usize = 8;
    const O_MIGRATABLE_TPC: usize = 10;
    const O_UGPU: usize = 12;
    const O_PHYS_CPC: usize = 14;
    const O_VIRTUAL_TPC: usize = 16;
    const _: () = assert!(O_VIRTUAL_TPC + 2 == SM_ENTRY_SIZE);

    let mut sm = 0usize;
    for (global_tpc, t) in p.tpcs.iter().enumerate() {
        for local_sm in 0..p.sms_per_tpc {
            let at = sm * SM_ENTRY_SIZE;
            let e = &mut out[at..at + SM_ENTRY_SIZE];
            put16(e, O_GPC, t.gpc_id);
            put16(e, O_LOCAL_TPC, t.local_tpc_id);
            put16(e, O_LOCAL_SM, local_sm);
            // `global_tpc` is bounded by `validate()`'s `tpcs.len() <= GR_MAX_SM`.
            put16(
                e,
                O_GLOBAL_TPC,
                u16::try_from(global_tpc).unwrap_or(u16::MAX),
            );
            put16(e, O_VIRTUAL_GPC, 0);
            put16(e, O_MIGRATABLE_TPC, 0);
            put16(e, O_UGPU, 0);
            put16(e, O_PHYS_CPC, 0);
            put16(e, O_VIRTUAL_TPC, t.virtual_tpc_id);
            sm += 1;
        }
    }
    put16(&mut out, GR_MAX_SM * SM_ENTRY_SIZE, num_sm);
    put16(&mut out, GR_MAX_SM * SM_ENTRY_SIZE + 2, num_tpc);
    Ok(out)
}

/// `NV2080_CTRL_INTERNAL_STATIC_GR_GET_FECS_RECORD_SIZE_PARAMS`.
///
/// # Errors
/// [`GrStaticError::FecsRecordSizeZero`] — and it is a refusal rather than a clamp, because
/// zero is what `fecsBufferMap`'s record-count division reads.
pub fn encode_fecs_record_size(p: &GrStaticProfile) -> Result<Vec<u8>, GrStaticError> {
    p.validate()?;
    let mut out = vec![0u8; FECS_RECORD_SIZE_PARAMS_SIZE];
    put32(&mut out, 0, p.fecs_record_size);
    Ok(out)
}

/// `NV2080_CTRL_INTERNAL_STATIC_GR_GET_PDB_PROPERTIES_PARAMS` — one `NvBool` per engine.
///
/// # Errors
/// [`GrStaticError`] if the profile is not self-consistent.
pub fn encode_pdb_properties(p: &GrStaticProfile) -> Result<Vec<u8>, GrStaticError> {
    p.validate()?;
    let mut out = vec![0u8; PDB_PROPERTIES_PARAMS_SIZE];
    out[0] = u8::from(p.per_subctx_header_supported);
    Ok(out)
}

// ═══════════════════════════════════════════════════════════════════════════════════════
// The DEFERRED half — `0x20800a32`, and why it needed a boot to be sure of
// ═══════════════════════════════════════════════════════════════════════════════════════

//
// ★★★ This section exists because a boot answered a question this module had deliberately
// left open. The header above says of `NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_CONTEXT_-
// BUFFERS_INFO`:
//
//   "`[assumed]`, and named as such: this port has not established which way
//    `kgraphicsShouldDeferContextInit` goes on GA106, and it is deliberately *not* being
//    guessed at, because the boot settles it for free."
//
// `[measured]` run `stateload1`, a stock 580.159.04 guest at `041b4f1`:
//
//   NVRM: nvCheckOkFailedNoLog: Check failed: NV_ERR_NOT_SUPPORTED (0x56) returned from
//         pRmApi->Control(..., NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_CONTEXT_BUFFERS_INFO,
//         pParams, sizeof(*pParams)) @ kernel_graphics.c:743
//   NVRM: ... returned from kgraphicsInitializeDeferredStaticData(...) @ kernel_graphics.c:1527
//
// ⇒ `kgraphicsShouldDeferContextInit` is **FALSE** on this chip, the `:1524-1529` branch IS
// taken, and `0x20800a32` is the **sixth** mandatory GR static-info control. ⊘ The guess
// would have gone either way; the boot cost nothing extra because the two outcomes are
// distinguishable in the log, which is the whole reason it was left open rather than
// resolved by argument.

/// `NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_CONTEXT_BUFFERS_INFO`
/// (`ogkm-580: ctrl/ctrl2080/ctrl2080internal.h:530`).
pub const NV2080_CTRL_CMD_INTERNAL_STATIC_KGR_GET_CONTEXT_BUFFERS_INFO: u32 = 0x2080_0a32;

/// `NV0080_CTRL_FIFO_GET_ENGINE_CONTEXT_PROPERTIES_ENGINE_ID_COUNT`
/// (`ogkm-580: ctrl/ctrl0080/ctrl0080fifo.h:147`) — 26, and the array bound of
/// `NV2080_CTRL_INTERNAL_STATIC_GR_CONTEXT_BUFFERS_INFO`.
pub const CONTEXT_BUFFER_ID_COUNT: usize = 0x1a;

/// `sizeof(NV2080_CTRL_INTERNAL_STATIC_GR_GET_CONTEXT_BUFFERS_INFO_PARAMS)` —
/// `8 * 26 * (size + alignment)`.
pub const CONTEXT_BUFFERS_INFO_PARAMS_SIZE: usize = GR_MAX_ENGINES * CONTEXT_BUFFER_ID_COUNT * 8;

/// ★★★ `NV_U32_MAX` is **"this buffer does not exist on this chip"**, not a very large
/// buffer.
///
/// `kgraphicsInitializeDeferredStaticData`'s consumer tests for it by hand:
/// `if (pKernelGraphicsStaticInfo->pContextBuffersInfo->engine[i].size != NV_U32_MAX)`
/// (`ogkm-580: kernel_graphics.c:2485`). ⊘ So a row this port does not know must be
/// `ABSENT` and never `0` — zero is a real, allocatable, empty buffer and two of GA106's
/// rows genuinely are zero (`GFXP_POOL` and `SETUP`), which is exactly why the two spellings
/// cannot be merged.
pub const CONTEXT_BUFFER_ABSENT: u32 = u32::MAX;

/// One context buffer's size and alignment, indexed by
/// `NV0080_CTRL_FIFO_GET_ENGINE_CONTEXT_PROPERTIES_ENGINE_ID_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextBuffer {
    /// Bytes, or [`CONTEXT_BUFFER_ABSENT`].
    pub size: u32,
    /// Required alignment in bytes.
    pub alignment: u32,
}

const ABSENT: ContextBuffer = ContextBuffer {
    size: CONTEXT_BUFFER_ABSENT,
    alignment: CONTEXT_BUFFER_ABSENT,
};
const fn buf(size: u32, alignment: u32) -> ContextBuffer {
    ContextBuffer { size, alignment }
}

/// GA106's twenty-six context buffers, in `ENGINE_ID` order — the index **is** the id, and
/// the names in the comments are `ctrl0080fifo.h:121-146`.
///
/// `[measured]` from `C: mode2_initctrl_ga106.h`'s `ctl_20800a32` (`psize` 1664, `dlen`
/// 1664 — nothing trimmed). ⚠ Ids 1-7 are the non-graphics engines (`VLD`, `VIDEO`, `MPEG`,
/// `CAPTURE`, `DISPLAY`, `ENCRYPTION`, `POSTPROCESS`) and a real GA106 reports every one of
/// them [`CONTEXT_BUFFER_ABSENT`].
pub const GA106_CONTEXT_BUFFERS: [ContextBuffer; CONTEXT_BUFFER_ID_COUNT] = [
    buf(0x000a_9700, 0x1000), // 0x00 GRAPHICS
    ABSENT,                   // 0x01 VLD
    ABSENT,                   // 0x02 VIDEO
    ABSENT,                   // 0x03 MPEG
    ABSENT,                   // 0x04 CAPTURE
    ABSENT,                   // 0x05 DISPLAY
    ABSENT,                   // 0x06 ENCRYPTION
    ABSENT,                   // 0x07 POSTPROCESS
    buf(0x0007_8600, 0x1000), // 0x08 GRAPHICS_ZCULL
    buf(0x0000_8600, 0x1000), // 0x09 GRAPHICS_PM
    buf(0x0070_0000, 0x1000), // 0x0a COMPUTE_PREEMPT
    buf(0x0011_8c00, 0x1000), // 0x0b GRAPHICS_PREEMPT
    buf(0x0010_6800, 0x0100), // 0x0c GRAPHICS_SPILL
    buf(0x0002_0000, 0x0100), // 0x0d GRAPHICS_PAGEPOOL
    buf(0x0032_9a80, 0x1000), // 0x0e GRAPHICS_BETACB
    buf(0x0008_2000, 0x0100), // 0x0f GRAPHICS_RTV
    buf(0x0000_4000, 0x1000), // 0x10 GRAPHICS_PATCH
    buf(0x0000_3000, 0x0100), // 0x11 GRAPHICS_BUNDLE_CB
    buf(0x0002_0000, 0x0100), // 0x12 GRAPHICS_PAGEPOOL_GLOBAL
    buf(0x0085_1200, 0x1000), // 0x13 GRAPHICS_ATTRIBUTE_CB
    buf(0x0008_0000, 0x0100), // 0x14 GRAPHICS_RTV_CB_GLOBAL
    buf(0x0000_0000, 0x1000), // 0x15 GRAPHICS_GFXP_POOL   ⚠ zero, and PRESENT
    buf(0x0000_0464, 0x1000), // 0x16 GRAPHICS_GFXP_CTRL_BLK
    buf(0x0001_0000, 0x1000), // 0x17 GRAPHICS_FECS_EVENT
    buf(0x0008_0000, 0x1000), // 0x18 GRAPHICS_PRIV_ACCESS_MAP
    buf(0x0000_0000, 0x1000), // 0x19 GRAPHICS_SETUP       ⚠ zero, and PRESENT
];

/// `NV2080_CTRL_INTERNAL_STATIC_GR_GET_CONTEXT_BUFFERS_INFO_PARAMS`.
///
/// ⚠ Unlike the other five this is **not** a function of [`GrStaticProfile`]'s geometry —
/// the sizes are the chip's own context-buffer table, so they are passed in directly. That
/// asymmetry is deliberate: pretending these could be derived from GPC and TPC counts would
/// be inventing a relationship this port has not established.
///
/// # Errors
/// [`GrStaticError::ContextBufferAlignmentZero`] for a present buffer with no alignment —
/// `memdescAlloc`'s alignment argument is not permitted to be zero, and a zero here would
/// reach it.
pub fn encode_context_buffers_info(
    buffers: &[ContextBuffer; CONTEXT_BUFFER_ID_COUNT],
) -> Result<Vec<u8>, GrStaticError> {
    let mut out = vec![0u8; CONTEXT_BUFFERS_INFO_PARAMS_SIZE];
    for (i, b) in buffers.iter().enumerate() {
        if b.size != CONTEXT_BUFFER_ABSENT && b.alignment == 0 {
            return Err(GrStaticError::ContextBufferAlignmentZero { id: i });
        }
        put32(&mut out, i * 8, b.size);
        put32(&mut out, i * 8 + 4, b.alignment);
    }
    Ok(out)
}
