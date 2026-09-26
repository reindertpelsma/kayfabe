//! ★★★ **The GR static-info differential** — every byte this port would send for the five
//! structurally mandatory GR controls, compared against what a **real GA106's own GSP
//! actually replied**.
//!
//! # Why this is a test and not a tautology
//!
//! The fixtures under `tests/fixtures/` are extracted verbatim from the C research
//! artifact's captured init-control table — *"GA106 init `GSP_RM_CONTROL` responses (real,
//! captured from host)"*, `C: src/qemu/mode2_initctrl_ga106.h:1-2`, `nvidia-gpu-passthrough`
//! rev `8baf4f2`. They were **not** produced by this port.
//!
//! `kf_abi::grstatic` does not carry them. It carries a *geometry* — three GPC rows,
//! fourteen TPC rows, two SMs per TPC, a record size, a boolean — and encoders that lay that
//! geometry out according to `ogkm-580`'s struct declarations. This file is where the two
//! meet. ⇒ a wrong field offset, a wrong array stride, a wrong bound, a wrong endianness, a
//! wrong SM pairing or a wrong GPC ordering all fail here, and every one of those would
//! otherwise have produced a *plausible* reply the guest silently believed.
//!
//! ⊘ What it does NOT establish: that these are the right numbers **for whatever GPU the
//! reader has**. They are a GA106's, and this device presents a GA106. See
//! `kf_abi::grstatic`'s header.

use kf_abi::oracle as abi_oracle;

use kf_abi::grstatic::{
    FECS_RECORD_SIZE_PARAMS_SIZE, FLOORSWEEPING_PARAMS_SIZE, GA106_GR_STATIC, GR_CAPS_PARAMS_SIZE,
    GR_MAX_ENGINES, GR_MAX_SM, GpcRow, GrStaticError, GrStaticProfile, PDB_PROPERTIES_PARAMS_SIZE,
    SM_ENTRY_SIZE, SM_ORDER_PARAMS_SIZE, SM_ORDER_ROW_SIZE, TpcRow, encode_fecs_record_size,
    encode_floorsweeping_masks, encode_global_sm_order, encode_gr_caps, encode_pdb_properties,
};

fn oracle(name: &str) -> Vec<u8> {
    let p = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&p).unwrap_or_else(|e| panic!("fixture {p} unreadable: {e}"))
}

/// ★ The oracle's `dlen` for `0x20800a22` is 16 376 of a 34 592-byte `psize` — the C's
/// recorder kept one message-queue element. Compare exactly the prefix it kept, and say so,
/// rather than padding the fixture with zeros this port supplied and calling that agreement.
///
/// ★★★ `cmd` is what makes the prose above **checkable**. This helper used to take the
/// oracle's bytes on trust: whatever length the fixture happened to be became "the prefix",
/// so a fixture that silently grew — by a re-extraction that zero-padded to `psize`, say —
/// would have widened the claim without a word changing. Now the length is checked against
/// [`kf_abi::oracle`]'s censused `kept` for that control, and a comparison that reaches
/// past what the recorder kept is refused by name.
fn assert_prefix_matches_for(cmd: u32, what: &str, ours: &[u8], theirs: &[u8]) {
    // The oracle's own account of this row — truncated or complete — decides how many bytes
    // the fixture is allowed to be evidence for.
    let kept = match abi_oracle::truncated_row(cmd) {
        Some(r) => r.kept,
        None => theirs.len(),
    };
    assert_eq!(
        theirs.len(),
        kept,
        "{what} ({cmd:#010x}): the fixture is {} bytes but the C recorder kept {kept}. \
         A fixture longer than `dlen` is this port's zero-fill wearing the oracle's name.",
        theirs.len()
    );
    assert!(
        abi_oracle::field_is_captured(0, theirs.len(), kept),
        "{what} ({cmd:#010x}): comparing {} bytes reaches past the captured prefix",
        theirs.len()
    );
    // ★★★ And the extent this file relies on must be the extent `kf_abi::oracle`
    // publishes for it. Until 2026-08-02 the predicate above was the ONLY call site of
    // `field_is_captured` in the tree, so it proved this file sound and said nothing about
    // the other eight truncated rows; the reliance table is where that gap was closed, and
    // this equality is what keeps the two from drifting apart.
    if let Some(r) = abi_oracle::capture_reliance(cmd) {
        assert_eq!(
            r.read_end,
            theirs.len(),
            "{what} ({cmd:#010x}): this file compares {} bytes but the reliance statement \
             says the argument rests on {}",
            theirs.len(),
            r.read_end
        );
    }
    assert_prefix_matches(what, ours, theirs);
}

fn assert_prefix_matches(what: &str, ours: &[u8], theirs: &[u8]) {
    assert!(
        ours.len() >= theirs.len(),
        "{what}: our reply is {} bytes, shorter than the oracle's captured {} — the oracle \
         cannot be a prefix of something smaller than itself",
        ours.len(),
        theirs.len()
    );
    if ours[..theirs.len()] == *theirs {
        return;
    }
    let at = ours
        .iter()
        .zip(theirs.iter())
        .position(|(a, b)| a != b)
        .unwrap_or(0);
    panic!(
        "{what}: first difference at byte {at} (0x{at:x}) — ours 0x{:02x}, the real GA106 \
         GSP's 0x{:02x}. ⊘ This is a layout or a value disagreement with real silicon, not \
         a flaky test.",
        ours[at], theirs[at]
    );
}

#[test]
fn gr_caps_matches_the_real_ga106_reply() {
    let ours = encode_gr_caps(&GA106_GR_STATIC).expect("GA106 profile encodes");
    assert_eq!(
        ours.len(),
        GR_CAPS_PARAMS_SIZE,
        "psize must be 8 engines' worth"
    );
    assert_eq!(ours.len(), 184, "the caller passes sizeof(pParams->caps)");
    assert_prefix_matches_for(
        0x2080_0a1f,
        "GR caps",
        &ours,
        &oracle("ga106_ctl_20800a1f.bin"),
    );
}

#[test]
fn floorsweeping_masks_match_the_real_ga106_reply() {
    let ours = encode_floorsweeping_masks(&GA106_GR_STATIC).expect("GA106 profile encodes");
    assert_eq!(ours.len(), FLOORSWEEPING_PARAMS_SIZE);
    assert_eq!(ours.len(), 3008);
    assert_prefix_matches_for(
        0x2080_0a26,
        "GR floorsweeping masks",
        &ours,
        &oracle("ga106_ctl_20800a26.bin"),
    );
}

#[test]
fn global_sm_order_matches_the_real_ga106_reply_over_the_captured_prefix() {
    let ours = encode_global_sm_order(&GA106_GR_STATIC).expect("GA106 profile encodes");
    assert_eq!(ours.len(), SM_ORDER_PARAMS_SIZE);
    assert_eq!(ours.len(), 34592, "8 engines x (240*18 + 4)");
    let theirs = oracle("ga106_ctl_20800a22.bin");
    assert_eq!(
        theirs.len(),
        16376,
        "the oracle's own dlen — if this changes the comparison below silently covers a \
         different amount of the reply"
    );
    assert_prefix_matches_for(0x2080_0a22, "GR global SM order", &ours, &theirs);
    // ★ Engine 0 is entirely inside the captured prefix, which is the reason the truncation
    // costs nothing: `grIdx` is 0 and nothing reads engines 1..7.
    assert!(
        SM_ORDER_ROW_SIZE < theirs.len(),
        "engine 0 ({SM_ORDER_ROW_SIZE} bytes) must be inside the captured prefix"
    );
}

#[test]
fn fecs_record_size_matches_the_real_ga106_reply() {
    let ours = encode_fecs_record_size(&GA106_GR_STATIC).expect("GA106 profile encodes");
    assert_eq!(ours.len(), FECS_RECORD_SIZE_PARAMS_SIZE);
    assert_prefix_matches_for(
        0x2080_0a3d,
        "FECS record size",
        &ours,
        &oracle("ga106_ctl_20800a3d.bin"),
    );
}

#[test]
fn pdb_properties_match_the_real_ga106_reply() {
    let ours = encode_pdb_properties(&GA106_GR_STATIC).expect("GA106 profile encodes");
    assert_eq!(ours.len(), PDB_PROPERTIES_PARAMS_SIZE);
    assert_prefix_matches_for(
        0x2080_0a48,
        "PDB properties",
        &ours,
        &oracle("ga106_ctl_20800a48.bin"),
    );
}

/// ★★★ The rejected shortcut, made unreachable rather than merely discouraged.
///
/// `_kgraphicsPostSchedulingEnableHandler` returns `NV_OK` early when `gpcMask == 0`
/// (`ogkm-580: kernel_graphics.c:486`). A profile with no GPCs is the only way to produce
/// that mask, and it is refused **by name** — so the shortcut cannot be taken by accident,
/// by a later edit, or by a chip row somebody left half-filled.
#[test]
fn a_profile_with_no_gpcs_cannot_encode_the_zero_gpc_mask_shortcut() {
    static EMPTY: [GpcRow; 0] = [];
    let p = GrStaticProfile {
        gpcs: &EMPTY,
        ..GA106_GR_STATIC
    };
    assert_eq!(
        p.gpc_mask(),
        Err(GrStaticError::GpcCountOutOfRange { count: 0 })
    );
    assert!(encode_floorsweeping_masks(&p).is_err());
    assert!(encode_gr_caps(&p).is_err());
}

/// The five replies are five views of one geometry, and RM cross-checks them. A profile in
/// which `tpcCount` and `tpcMask` disagree must not encode at all — in **any** of the five,
/// not merely in the one that carries the masks.
#[test]
fn a_mask_that_disagrees_with_its_count_is_refused_by_every_encoder() {
    static BAD: [GpcRow; 1] = [GpcRow {
        physical_id: 0,
        tpc_mask: 0b111,
        tpc_count: 4, // three bits set, four claimed
        mmu_per_gpc: 1,
        num_pes_per_gpc: 1,
        zcull_mask: 0xf,
        ppc_mask: None,
        rop_mask: None,
    }];
    static TPCS: [TpcRow; 4] = [TpcRow::plain(0, 0, 0); 4];
    let p = GrStaticProfile {
        gpcs: &BAD,
        tpcs: &TPCS,
        ..GA106_GR_STATIC
    };
    let want = Err(GrStaticError::TpcMaskCountMismatch {
        gpc: 0,
        mask: 0b111,
        count: 4,
    });
    assert_eq!(encode_gr_caps(&p), want);
    assert_eq!(encode_floorsweeping_masks(&p), want);
    assert_eq!(encode_global_sm_order(&p), want);
    assert_eq!(encode_fecs_record_size(&p), want);
    assert_eq!(encode_pdb_properties(&p), want);
}

/// `numGfxTpc` is summed from the GPC rows while `numTpc` is the length of the TPC list.
/// They describe the same silicon, so a profile in which they differ is refused rather than
/// encoded with whichever one the reader happens to look at.
#[test]
fn tpc_rows_that_do_not_add_up_to_the_gpc_counts_are_refused() {
    static ONE: [TpcRow; 1] = [TpcRow::plain(0, 0, 0)];
    let p = GrStaticProfile {
        tpcs: &ONE,
        ..GA106_GR_STATIC
    };
    assert_eq!(
        encode_global_sm_order(&p),
        Err(GrStaticError::TpcRowsDoNotMatchGpcCounts {
            from_gpcs: 14,
            rows: 1
        })
    );
}

/// A TPC row naming a GPC that is not in the profile would encode a `gpcId` RM indexes
/// arrays with.
#[test]
fn a_tpc_row_naming_a_nonexistent_gpc_is_refused() {
    static ROWS: [TpcRow; 14] = [TpcRow::plain(9, 0, 0); 14];
    let p = GrStaticProfile {
        tpcs: &ROWS,
        ..GA106_GR_STATIC
    };
    assert_eq!(
        encode_global_sm_order(&p),
        Err(GrStaticError::GpcIdOutOfRange { row: 0, gpc_id: 9 })
    );
}

/// `fecsRecordSize` is a divisor in `fecsBufferMap`'s record-count arithmetic, so zero is a
/// divide rather than a small buffer.
#[test]
fn a_zero_fecs_record_size_is_refused() {
    let p = GrStaticProfile {
        fecs_record_size: 0,
        ..GA106_GR_STATIC
    };
    assert_eq!(
        encode_fecs_record_size(&p),
        Err(GrStaticError::FecsRecordSizeZero)
    );
}

/// ⊘ The property the fixture comparison cannot state, because the capture is truncated
/// before engines 1..7: **only engine 0 is described, and the rest are zero.** RM reads
/// `engineCaps[grIdx]` with `grIdx = 0`; anything non-zero elsewhere would be this port
/// inventing a second GR engine.
#[test]
fn only_engine_zero_is_described_in_every_reply() {
    let caps = encode_gr_caps(&GA106_GR_STATIC).unwrap();
    assert!(
        caps[GR_CAPS_PARAMS_SIZE / GR_MAX_ENGINES..]
            .iter()
            .all(|b| *b == 0),
        "GR caps: engines 1..7 must be zero"
    );
    let fs = encode_floorsweeping_masks(&GA106_GR_STATIC).unwrap();
    assert!(
        fs[FLOORSWEEPING_PARAMS_SIZE / GR_MAX_ENGINES..]
            .iter()
            .all(|b| *b == 0),
        "floorsweeping: engines 1..7 must be zero"
    );
    let sm = encode_global_sm_order(&GA106_GR_STATIC).unwrap();
    assert!(
        sm[SM_ORDER_ROW_SIZE..].iter().all(|b| *b == 0),
        "SM order: engines 1..7 must be zero — this is the half of the 34 592 bytes the \
         oracle's capture could not pin"
    );
    let fecs = encode_fecs_record_size(&GA106_GR_STATIC).unwrap();
    assert!(fecs[4..].iter().all(|b| *b == 0));
    let pdb = encode_pdb_properties(&GA106_GR_STATIC).unwrap();
    assert!(pdb[1..].iter().all(|b| *b == 0));
}

/// The SM-order tail past `numSm` must be zero, and `numSm`/`numTpc` must sit at the very
/// end of engine 0's row rather than after the last *used* entry — a natural off-by-a-lot
/// that the truncated fixture alone would not catch, because both spellings agree over the
/// first 4 320 bytes.
#[test]
fn num_sm_and_num_tpc_sit_after_the_full_240_entry_array() {
    let sm = encode_global_sm_order(&GA106_GR_STATIC).unwrap();
    let used = GA106_GR_STATIC.tpcs.len() * GA106_GR_STATIC.sms_per_tpc as usize;
    assert_eq!(used, 28);
    assert!(
        sm[used * SM_ENTRY_SIZE..GR_MAX_SM * SM_ENTRY_SIZE]
            .iter()
            .all(|b| *b == 0),
        "entries 28..240 must be zero"
    );
    let at = GR_MAX_SM * SM_ENTRY_SIZE;
    assert_eq!(u16::from_le_bytes([sm[at], sm[at + 1]]), 28, "numSm");
    assert_eq!(u16::from_le_bytes([sm[at + 2], sm[at + 3]]), 14, "numTpc");
}

// ═══════ the deferred half — the control a boot, not an argument, proved mandatory ═══════
//
// ★ The boot is `stateload1`, rev `041b4f1`, 2026-08-01, evidence at
// `/workspace/bench/run_stateload1_dmesg.log`. See `docs/design/boot_measured_2026_08_01.md`
// §39, and `kf_abi::grstatic`'s deferred-half section for the reading it settled.

use kf_abi::grstatic::{
    CONTEXT_BUFFER_ABSENT, CONTEXT_BUFFER_ID_COUNT, CONTEXT_BUFFERS_INFO_PARAMS_SIZE,
    ContextBuffer, GA106_CONTEXT_BUFFERS, encode_context_buffers_info,
};

#[test]
fn context_buffers_info_matches_the_real_ga106_reply() {
    let ours = encode_context_buffers_info(&GA106_CONTEXT_BUFFERS).expect("GA106 encodes");
    assert_eq!(ours.len(), CONTEXT_BUFFERS_INFO_PARAMS_SIZE);
    assert_eq!(
        ours.len(),
        1664,
        "8 engines x 26 buffers x (size + alignment)"
    );
    assert_prefix_matches(
        "GR context buffers info",
        &ours,
        &oracle("ga106_ctl_20800a32.bin"),
    );
}

/// ★★★ `NV_U32_MAX` means **absent** and `0` means **present and empty**, and the two are
/// not interchangeable: RM tests for the sentinel by hand
/// (`ogkm-580: kernel_graphics.c:2485`), so collapsing them would make two real GA106
/// buffers — `GFXP_POOL` and `SETUP` — vanish from the allocation walk.
///
/// ⊘ This is the property the fixture comparison alone cannot state, because a table that
/// wrote `0` for the absent rows would also have to differ from the capture somewhere else
/// to be caught. Here it is stated directly.
#[test]
fn absent_and_empty_are_different_context_buffers() {
    assert_eq!(CONTEXT_BUFFER_ABSENT, u32::MAX);
    let absent: Vec<usize> = (0..CONTEXT_BUFFER_ID_COUNT)
        .filter(|i| GA106_CONTEXT_BUFFERS[*i].size == CONTEXT_BUFFER_ABSENT)
        .collect();
    assert_eq!(
        absent,
        vec![1, 2, 3, 4, 5, 6, 7],
        "VLD, VIDEO, MPEG, CAPTURE, DISPLAY, ENCRYPTION, POSTPROCESS — the non-graphics \
         engines a GA106 does not carry a GR context buffer for"
    );
    let empty: Vec<usize> = (0..CONTEXT_BUFFER_ID_COUNT)
        .filter(|i| GA106_CONTEXT_BUFFERS[*i].size == 0)
        .collect();
    assert_eq!(
        empty,
        vec![0x15, 0x19],
        "GFXP_POOL and SETUP are PRESENT with size 0 — if these ever read as absent the \
         two spellings have been collapsed"
    );
    assert_eq!(
        GA106_CONTEXT_BUFFERS[0].size, 0x000a_9700,
        "ENGINE_ID_GRAPHICS, the one kgraphicsAllocGrGlobalCtxBuffers sizes the main \
         context buffer from"
    );
}

/// A present buffer with no alignment reaches `memdescAlloc` as a zero alignment.
#[test]
fn a_present_context_buffer_with_zero_alignment_is_refused() {
    let mut t = GA106_CONTEXT_BUFFERS;
    t[0] = ContextBuffer {
        size: 0x1000,
        alignment: 0,
    };
    assert_eq!(
        encode_context_buffers_info(&t),
        Err(GrStaticError::ContextBufferAlignmentZero { id: 0 })
    );
    // ⚠ …but the ABSENT rows carry `alignment == NV_U32_MAX`, not a sane alignment, and a
    // checker that demanded one from them would refuse real hardware.
    assert!(encode_context_buffers_info(&GA106_CONTEXT_BUFFERS).is_ok());
}

// ═══════ floorswept parts — the placement, against REAL GSP replies (2026-09-26) ═══════
//
// ★★★ The fixture above is a GA106 whose logical order IS its physical order and whose mask is
// contiguous, so it cannot tell a row-position encoder from a physical-id one. These are the
// replies that can: extracted verbatim by `scripts/rpctrace/decode_rpctrace.py` (via
// `ctrl_payload_pairs.py`, both sessions byte-identical) from
//
// | fixture | trace (md5) | seqs | why it matters |
// |---|---|---|---|
// | `ga106b_rpctrace580_ctl_20800a2{6,2}.bin` | `traces/rpctrace_ga106_boot1.bin` (`0fcc24c7…`), 580.159.04 — the GUEST's driver | 131/610, 133/612 | a second RTX 3060 whose four-TPC GPC is PHYSICAL 1: logical ≠ physical on a contiguous mask |
// | `ad102_rpctrace575_ctl_20800a2{6,2}.bin` | `traces/ad102_boot1.bin` (`751840ae…`), 575.51.03 | 132/605, 134/607 | an RTX 4090: `gpcMask = 0xffe`, physical GPC 0 fused |
//
// ⚠ The AD102 replies are 575's layout — `NV2080_CTRL_INTERNAL_GR_MAX_GPC` was 12 there (2368 =
// 8 × 296 bytes) and the SM entry had 7 fields (26912 = 8 × (240 × 14 + 4)) — so it is compared
// FIELD BY FIELD against our 580 encoding, not byte for byte.

use kf_abi::grstatic::{GR_MAX_GPC, MAX_TPC_PER_GPC};

/// One decoded `NV2080_CTRL_INTERNAL_STATIC_GR_FLOORSWEEPING_MASKS` row (engine 0), for a
/// layout whose GPC arrays have `g` slots.
#[derive(Debug, PartialEq, Eq)]
struct Fs {
    gpc_mask: u32,
    tpc_mask: Vec<u32>,
    tpc_count: Vec<u32>,
    phys_gpc_mask: u32,
    mmu_per_gpc: Vec<u32>,
    tpc_to_pes: Vec<u32>,
    num_pes: Vec<u32>,
    zcull: Vec<u32>,
    phys_gfx_gpc_mask: u32,
    num_gfx_tpc: u32,
}

fn decode_fs(b: &[u8], g: usize) -> Fs {
    let w = |i: usize| u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap());
    let arr = |at: usize, n: usize| (at..at + n).map(w).collect::<Vec<u32>>();
    let mut at = 0;
    let mut take = |n: usize| {
        let v = arr(at, n);
        at += n;
        v
    };
    let gpc_mask = take(1)[0];
    let tpc_mask = take(g);
    let tpc_count = take(g);
    let phys_gpc_mask = take(1)[0];
    let mmu_per_gpc = take(g);
    let tpc_to_pes = take(MAX_TPC_PER_GPC);
    let num_pes = take(g);
    let zcull = take(g);
    let phys_gfx_gpc_mask = take(1)[0];
    let num_gfx_tpc = take(1)[0];
    Fs {
        gpc_mask,
        tpc_mask,
        tpc_count,
        phys_gpc_mask,
        mmu_per_gpc,
        tpc_to_pes,
        num_pes,
        zcull,
        phys_gfx_gpc_mask,
        num_gfx_tpc,
    }
}

/// The TPC rows of an SM-order reply (engine 0), `nf` `NvU16` fields per entry — 9 in 580
/// (`gpcId, localTpcId, localSmId, globalTpcId, virtualGpcId, migratableTpcId, ugpuId,
/// physicalCpcId, virtualTpcId`), 7 in 575 (the last two absent).
fn decode_sm_order(b: &[u8], nf: usize) -> (Vec<TpcRow>, u16) {
    let h = |i: usize| u16::from_le_bytes([b[2 * i], b[2 * i + 1]]);
    let num_sm = h(240 * nf) as usize;
    let num_tpc = h(240 * nf + 1) as usize;
    let f = |sm: usize, k: usize| if k < nf { h(sm * nf + k) } else { 0 };
    let tpcs = (0..num_tpc)
        .map(|t| {
            let sm = (0..num_sm)
                .find(|&sm| f(sm, 3) as usize == t && f(sm, 2) == 0)
                .expect("localSmId 0");
            TpcRow {
                gpc_id: f(sm, 0),
                local_tpc_id: f(sm, 1),
                virtual_tpc_id: f(sm, 8),
                virtual_gpc_id: f(sm, 4),
                migratable_tpc_id: f(sm, 5),
                ugpu_id: f(sm, 6),
                physical_cpc_id: f(sm, 7),
            }
        })
        .collect();
    (tpcs, (num_sm / num_tpc) as u16)
}

/// The rows a host's per-index controls produce from a reply: logical GPC `l` is `map[l]`.
fn rows_from(fs: &Fs, map: &[u32]) -> Vec<GpcRow> {
    map.iter()
        .enumerate()
        .map(|(l, &p)| GpcRow {
            physical_id: p,
            tpc_mask: fs.tpc_mask[p as usize],
            tpc_count: fs.tpc_count[l],
            mmu_per_gpc: fs.mmu_per_gpc[l],
            num_pes_per_gpc: fs.num_pes[l],
            zcull_mask: fs.zcull[p as usize],
            ppc_mask: None,
            rop_mask: None,
        })
        .collect()
}

fn profile_from(fs: &Fs, map: &[u32], tpcs: Vec<TpcRow>, sms_per_tpc: u16) -> GrStaticProfile {
    GrStaticProfile {
        gpcs: Box::leak(rows_from(fs, map).into_boxed_slice()),
        tpcs: Box::leak(tpcs.into_boxed_slice()),
        sms_per_tpc,
        tpc_to_pes_map: fs.tpc_to_pes.clone().try_into().unwrap(),
        gfx_gpc_mask: fs.phys_gfx_gpc_mask,
        num_gfx_tpc: fs.num_gfx_tpc,
        ..GA106_GR_STATIC
    }
}

/// ★★★ **A real GA106 whose logical order is NOT its physical order, reproduced byte for byte.**
///
/// `tpcMask` (physical) is `1f, 1b, 1f` and `tpcCount` (logical) `4, 5, 5`: logical GPC 0 is
/// physical GPC 1, the only four-TPC GPC. Rows in LOGICAL order naming their physical GPC
/// encode all 3 008 + 34 592 bytes exactly. ⊘ The pre-2026-09-26 encoder (rows in physical
/// order, `tpc_count = popcount`) served `tpcCount = 5, 4, 5` for this board — a wrong answer
/// no check caught, because the totals agree.
#[test]
fn a_real_ga106_whose_logical_order_is_not_physical_is_reproduced_byte_for_byte() {
    let fs_bytes = oracle("ga106b_rpctrace580_ctl_20800a26.bin");
    let sm_bytes = oracle("ga106b_rpctrace580_ctl_20800a22.bin");
    let fs = decode_fs(&fs_bytes, GR_MAX_GPC);
    assert_eq!(
        (fs.gpc_mask, &fs.tpc_mask[..3], &fs.tpc_count[..3]),
        (0x7, &[0x1f, 0x1b, 0x1f][..], &[4, 5, 5][..])
    );
    // Logical 0 → physical 1 (the four-TPC GPC); the two five-TPC GPCs are interchangeable
    // in this reply (same mask, same zcull), so the tie's order cannot change a byte.
    let map = [1, 0, 2];
    let (tpcs, sms) = decode_sm_order(&sm_bytes, 9);
    let p = profile_from(&fs, &map, tpcs, sms);
    p.validate().expect("the real board validates");
    assert_eq!(
        encode_floorsweeping_masks(&p).unwrap(),
        fs_bytes,
        "0x20800a26"
    );
    assert_eq!(encode_global_sm_order(&p).unwrap(), sm_bytes, "0x20800a22");
    // ⊘ And the old reading refuses to describe it: rows in physical order with the logical
    // counts disagree with their own masks.
    let physical_order = profile_from(&fs, &[0, 1, 2], decode_sm_order(&sm_bytes, 9).0, sms);
    assert!(matches!(
        physical_order.validate(),
        Err(GrStaticError::TpcMaskCountMismatch {
            gpc: 0,
            mask: 0x1f,
            count: 4
        })
    ));
}

/// ★★★ **An RTX 4090's `0xffe` — physical GPC 0 fused — placed as its own GSP placed it.**
///
/// Every array is compared field by field against the real reply (575's twelve-slot layout,
/// our sixteen-slot one zero past twelve): `tpcMask` and `zcullMask` at the PHYSICAL id (slot 0
/// zero for the fused GPC), `tpcCount`, `mmuPerGpc` and `numPesPerGpc` at the LOGICAL id
/// (slots 0..11 filled, slot 11 empty although physical GPC 11 is enabled).
#[test]
fn an_ad102_with_a_fused_gpc0_is_placed_as_its_own_gsp_placed_it() {
    let theirs = decode_fs(&oracle("ad102_rpctrace575_ctl_20800a26.bin"), 12);
    assert_eq!(theirs.gpc_mask, 0xffe, "physical GPC 0 fused");
    let (tpcs, sms) = decode_sm_order(&oracle("ad102_rpctrace575_ctl_20800a22.bin"), 7);
    assert_eq!((tpcs.len(), sms), (64, 2));
    // Logical 0..10 → physical 1..11 (physical 1 and 2 are the two five-TPC GPCs, and the
    // SM order's logical GPCs 0 and 1 are the two that hold five TPCs).
    let map: Vec<u32> = (1..=11).collect();
    let p = profile_from(&theirs, &map, tpcs, sms);
    p.validate().expect("the 4090 validates");
    assert_eq!(p.gpc_mask(), Ok(0xffe));
    let ours = decode_fs(&encode_floorsweeping_masks(&p).unwrap(), GR_MAX_GPC);
    let pad = |v: &[u32]| {
        let mut v = v.to_vec();
        v.resize(GR_MAX_GPC, 0);
        v
    };
    assert_eq!(ours.gpc_mask, theirs.gpc_mask);
    assert_eq!(ours.phys_gpc_mask, theirs.phys_gpc_mask);
    assert_eq!(ours.phys_gfx_gpc_mask, theirs.phys_gfx_gpc_mask);
    assert_eq!(ours.num_gfx_tpc, theirs.num_gfx_tpc);
    assert_eq!(ours.tpc_mask, pad(&theirs.tpc_mask), "tpcMask[] — physical");
    assert_eq!(ours.zcull, pad(&theirs.zcull), "zcullMask[] — physical");
    assert_eq!(
        ours.tpc_count,
        pad(&theirs.tpc_count),
        "tpcCount[] — logical"
    );
    assert_eq!(
        ours.mmu_per_gpc,
        pad(&theirs.mmu_per_gpc),
        "mmuPerGpc[] — logical"
    );
    assert_eq!(
        ours.num_pes,
        pad(&theirs.num_pes),
        "numPesPerGpc[] — logical"
    );
    assert_eq!(ours.tpc_to_pes, theirs.tpc_to_pes);
}

/// ★ The RTX 3060 Ti this fix was made on, from its host's own per-index answers
/// (`[measured 2026-09-26]`, `kf_abi::grstatic`'s table): `0x3e`, and fused slots zero.
#[test]
fn a_3060ti_publishes_its_own_non_contiguous_mask() {
    let row = |p: u32, m: u32| GpcRow {
        physical_id: p,
        tpc_mask: m,
        tpc_count: m.count_ones(),
        mmu_per_gpc: 1,
        num_pes_per_gpc: 2,
        zcull_mask: 0xf,
        ppc_mask: Some(3),
        rop_mask: Some(3),
    };
    let gpcs = [
        row(1, 0xe),
        row(2, 0xf),
        row(3, 0xf),
        row(4, 0xf),
        row(5, 0xf),
    ];
    // 19 TPCs: logical 0 holds three, the rest four each.
    let tpcs: Vec<TpcRow> = (0..5u16)
        .flat_map(|g| (0..if g == 0 { 3 } else { 4 }).map(move |t| TpcRow::plain(g, t, t)))
        .collect();
    let p = GrStaticProfile {
        gpcs: Box::leak(Box::new(gpcs)),
        tpcs: Box::leak(tpcs.into_boxed_slice()),
        gfx_gpc_mask: 0x3e,
        num_gfx_tpc: 19,
        ..GA106_GR_STATIC
    };
    p.validate().expect("validates");
    let fs = decode_fs(&encode_floorsweeping_masks(&p).unwrap(), GR_MAX_GPC);
    assert_eq!(
        (
            fs.gpc_mask,
            fs.phys_gpc_mask,
            fs.phys_gfx_gpc_mask,
            fs.num_gfx_tpc
        ),
        (0x3e, 0x3e, 0x3e, 19)
    );
    assert_eq!(fs.tpc_mask[..7], [0, 0xe, 0xf, 0xf, 0xf, 0xf, 0]);
    assert_eq!(fs.tpc_count[..6], [3, 4, 4, 4, 4, 0]);
    assert_eq!(fs.num_pes[..6], [2, 2, 2, 2, 2, 0]);
    assert_eq!(fs.zcull[..7], [0, 0xf, 0xf, 0xf, 0xf, 0xf, 0]);
}

/// ⊘ The new geometry refusals, each by name.
#[test]
fn physical_ids_and_per_gpc_tpc_rows_are_refused_by_name() {
    let mut rows = kf_abi::grstatic::GA106_GPCS;
    rows[2].physical_id = 1;
    let p = GrStaticProfile {
        gpcs: Box::leak(Box::new(rows)),
        ..GA106_GR_STATIC
    };
    assert_eq!(
        p.gpc_mask(),
        Err(GrStaticError::DuplicatePhysicalGpc {
            gpc: 2,
            physical_id: 1
        })
    );
    assert!(encode_floorsweeping_masks(&p).is_err());

    let mut rows = kf_abi::grstatic::GA106_GPCS;
    rows[0].physical_id = 16;
    let p = GrStaticProfile {
        gpcs: Box::leak(Box::new(rows)),
        ..GA106_GR_STATIC
    };
    assert_eq!(
        p.gpc_mask(),
        Err(GrStaticError::PhysicalGpcIdOutOfRange {
            gpc: 0,
            physical_id: 16
        })
    );

    // Totals agree (4 + 5 + 5) but GPC 0 is given a GPC 1 TPC: per-GPC, not only in sum.
    let mut tpcs = kf_abi::grstatic::GA106_TPCS;
    let i = tpcs
        .iter()
        .position(|t| t.gpc_id == 1 && t.local_tpc_id == 4)
        .unwrap();
    tpcs[i].gpc_id = 0;
    let p = GrStaticProfile {
        tpcs: Box::leak(Box::new(tpcs)),
        ..GA106_GR_STATIC
    };
    assert!(matches!(
        p.validate(),
        Err(GrStaticError::LocalTpcIdOutOfRange {
            gpc_id: 0,
            local_tpc_id: 4,
            ..
        })
    ));

    let p = GrStaticProfile {
        gfx_gpc_mask: 0b1000,
        ..GA106_GR_STATIC
    };
    assert_eq!(
        p.validate(),
        Err(GrStaticError::GfxGeometryOutOfRange {
            gfx_gpc_mask: 0b1000,
            num_gfx_tpc: 14
        })
    );
    let p = GrStaticProfile {
        num_gfx_tpc: 15,
        ..GA106_GR_STATIC
    };
    assert_eq!(
        p.validate(),
        Err(GrStaticError::GfxGeometryOutOfRange {
            gfx_gpc_mask: 0x7,
            num_gfx_tpc: 15
        })
    );
}
