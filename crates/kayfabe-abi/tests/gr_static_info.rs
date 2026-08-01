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
//! `kayfabe_abi::grstatic` does not carry them. It carries a *geometry* — three GPC rows,
//! fourteen TPC rows, two SMs per TPC, a record size, a boolean — and encoders that lay that
//! geometry out according to `ogkm-580`'s struct declarations. This file is where the two
//! meet. ⇒ a wrong field offset, a wrong array stride, a wrong bound, a wrong endianness, a
//! wrong SM pairing or a wrong GPC ordering all fail here, and every one of those would
//! otherwise have produced a *plausible* reply the guest silently believed.
//!
//! ⊘ What it does NOT establish: that these are the right numbers **for whatever GPU the
//! reader has**. They are a GA106's, and this device presents a GA106. See
//! `kayfabe_abi::grstatic`'s header.

use kayfabe_abi::grstatic::{
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
    assert_prefix_matches("GR caps", &ours, &oracle("ga106_ctl_20800a1f.bin"));
}

#[test]
fn floorsweeping_masks_match_the_real_ga106_reply() {
    let ours = encode_floorsweeping_masks(&GA106_GR_STATIC).expect("GA106 profile encodes");
    assert_eq!(ours.len(), FLOORSWEEPING_PARAMS_SIZE);
    assert_eq!(ours.len(), 3008);
    assert_prefix_matches(
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
    assert_prefix_matches("GR global SM order", &ours, &theirs);
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
    assert_prefix_matches("FECS record size", &ours, &oracle("ga106_ctl_20800a3d.bin"));
}

#[test]
fn pdb_properties_match_the_real_ga106_reply() {
    let ours = encode_pdb_properties(&GA106_GR_STATIC).expect("GA106 profile encodes");
    assert_eq!(ours.len(), PDB_PROPERTIES_PARAMS_SIZE);
    assert_prefix_matches("PDB properties", &ours, &oracle("ga106_ctl_20800a48.bin"));
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
        tpc_mask: 0b111,
        tpc_count: 4, // three bits set, four claimed
        mmu_per_gpc: 1,
        num_pes_per_gpc: 1,
        zcull_mask: 0xf,
    }];
    static TPCS: [TpcRow; 4] = [TpcRow {
        gpc_id: 0,
        local_tpc_id: 0,
        virtual_tpc_id: 0,
    }; 4];
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
    static ONE: [TpcRow; 1] = [TpcRow {
        gpc_id: 0,
        local_tpc_id: 0,
        virtual_tpc_id: 0,
    }];
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
    static ROWS: [TpcRow; 14] = [TpcRow {
        gpc_id: 9,
        local_tpc_id: 0,
        virtual_tpc_id: 0,
    }; 14];
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
