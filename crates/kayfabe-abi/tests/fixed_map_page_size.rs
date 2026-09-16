//! ★★★★★ **CONSTRAINT 28, half one — the page-size flag a FIXED map must carry.**
//!
//! `THE_CONSTRAINTS.md` §28: *"`NVOS46_FLAGS_DMA_OFFSET_FIXED_TRUE` honours an arbitrary VA
//! **only** with `NVOS46_FLAGS_PAGE_SIZE_4KB` — **0/3 without, 3/3 with** — and without it
//! RM **relocates and returns `NV_OK`**."*
//!
//! ⊘ **The rows below are the measurement, not an illustration.** They are the raw client's
//! own ring VAs as `traces/w744_b1d_probe/run3_FINAL_ga106_580.126.20.log` recorded them,
//! so a change to [`nvos46_page_size_flag`] that would have re-broken w744's 0/3 is red
//! here without anybody having to remember the number.
//!
//! ⚠ These tests say what flag we SEND. They say nothing about what RM does with it — that
//! is the hardware half, and its evidence is the trace cited above.

use kayfabe_abi::bringup::{
    NVOS46_BIG_PAGE_BYTES, NVOS46_FLAGS_PAGE_SIZE_4KB, nvos46_page_size_flag,
};

/// The three VAs w744 measured at `0/3 honoured` without the flag and `3/3` with it.
///
/// ⊘ w744 probed `0x90…`/`0x92…`/`0x94…` for the 4 KiB arm and `0x80…`/`0x82…`/`0x86…` for
/// the unflagged one — different bases, same shape: a `…_1000` offset inside a 2 MiB-aligned
/// region. Both sets are listed because the property under test is the `+0x1000`, and a test
/// that carried only one set would not show that.
const RING_VAS: &[u64] = &[
    0x0000_0080_0000_1000,
    0x0000_0082_0000_1000,
    0x0000_0086_0000_1000,
    0x0000_0090_0000_1000,
    0x0000_0092_0000_1000,
    0x0000_0094_0000_1000,
];

#[test]
fn every_ring_va_w744_measured_unhonoured_now_asks_for_the_small_page_table() {
    for &va in RING_VAS {
        // 4 KiB long, as a ring's own leaf is.
        let flag = nvos46_page_size_flag(va, 0, 0x1000);
        assert_eq!(
            flag,
            NVOS46_FLAGS_PAGE_SIZE_4KB,
            "★★★ CONSTRAINT 28 — a FIXED map at {va:#018x} must pin the small-page table. \
             `[measured w744]` without it RM answered NV_OK and placed the mapping at \
             {:#018x} instead, which is the `Xid 31 FAULT_PDE` reached through a success.",
            va & !(NVOS46_BIG_PAGE_BYTES - 1)
        );
    }
}

#[test]
fn a_big_aligned_request_leaves_the_page_size_to_rm() {
    // w744's informational row: `at=0x000000f000000000 … honoured=true` with NO flag.
    assert_eq!(
        nvos46_page_size_flag(0x0000_00f0_0000_0000, 0, NVOS46_BIG_PAGE_BYTES),
        0,
        "★ A big-aligned VA of a big-multiple length can be served by a big page AT ITS OWN \
         ADDRESS, so pinning it to 4 KiB would be pinning `pageSizeLockMask` for no reason \
         — see NVOS46_FLAGS_PAGE_SIZE_4KB's own warning about the lock mask."
    );
}

#[test]
fn a_big_aligned_base_with_a_ragged_length_still_pins_the_small_page_table() {
    // ⊘ THE HALF THAT IS EASY TO MISS. w277 recorded `0x8600`-long rows; a base that is big
    // aligned says nothing about whether the RANGE can be covered by big pages.
    assert_eq!(
        nvos46_page_size_flag(0x0000_00f0_0000_0000, 0, 0x8600),
        NVOS46_FLAGS_PAGE_SIZE_4KB,
        "★ Length is half the predicate. A 0x8600-byte range at a 64 KiB boundary is not a \
         whole number of big pages, so a big PTE would cover bytes the caller did not ask \
         for."
    );
}

#[test]
fn the_predicate_is_exactly_big_page_alignment_of_all_three_terms() {
    // ★ NON-VACUITY: the boundary is where it is claimed to be, on ALL THREE axes, and the
    // function is not simply "always 4KB" (which would pass the first three tests).
    // ⊘ w755: renamed from `..._of_both_terms`. It said `both` and checked two, while the
    // predicate took three from that commit on — a name is part of what a test asserts.
    assert_eq!(
        nvos46_page_size_flag(NVOS46_BIG_PAGE_BYTES, 0, NVOS46_BIG_PAGE_BYTES),
        0
    );
    assert_eq!(
        nvos46_page_size_flag(NVOS46_BIG_PAGE_BYTES - 0x1000, 0, NVOS46_BIG_PAGE_BYTES),
        NVOS46_FLAGS_PAGE_SIZE_4KB
    );
    assert_eq!(
        nvos46_page_size_flag(NVOS46_BIG_PAGE_BYTES, 0, NVOS46_BIG_PAGE_BYTES - 0x1000),
        NVOS46_FLAGS_PAGE_SIZE_4KB
    );
    assert_eq!(
        nvos46_page_size_flag(NVOS46_BIG_PAGE_BYTES, 0x1000, NVOS46_BIG_PAGE_BYTES),
        NVOS46_FLAGS_PAGE_SIZE_4KB,
        "the third axis — a slice's offset — belongs in the boundary check too"
    );
    assert_eq!(
        NVOS46_BIG_PAGE_BYTES, 65_536,
        "★ NON-VACUITY: the family big-page size moved. Every row above is stated in terms \
         of it, and a different value means the w744 rows are being checked against a \
         boundary hardware does not have."
    );
}

/// ★★★★★ **w755 — THE SLICE'S OFFSET IS THE THIRD QUANTITY, AND IT IS THE ONE THE SINGLE
/// STORE ADDED.**
///
/// ⊘ Every case above passes `offset = 0`, because every one of them was written when the
/// mapped object was a dedicated LEAF and its physical base was aligned by construction.
/// The single store maps a SLICE of one 11 904 MiB object, so the physical base is
/// `reservation_base + offset` and `offset` alone decides whether a big page can start
/// there.
///
/// ⚠ This is the exact shape that makes the old predicate return `0` on a run a guest
/// really produces: a 64 KiB-aligned guest VA whose backing frames start 4 KiB into a
/// 64 KiB block. VA alignment and frame alignment are INDEPENDENT.
#[test]
fn a_slice_at_a_small_aligned_offset_must_take_the_small_page_flag() {
    let big = NVOS46_BIG_PAGE_BYTES;

    // The case the old predicate got wrong: VA and length big-aligned, offset is not.
    assert_eq!(
        nvos46_page_size_flag(0x0000_00f0_0000_0000, 0x1000, big),
        NVOS46_FLAGS_PAGE_SIZE_4KB,
        "a slice starting 4 KiB into the store cannot be served by a 64 KiB page, however \
         well-aligned its VA is — this is the w753 `map_refused` mechanism"
    );

    // ... and it must still say "RM chooses" when all three really are big-aligned, or the
    // fix would be a blanket that pins every mapping to 4 KiB PTEs and costs the TLB.
    assert_eq!(
        nvos46_page_size_flag(0x0000_00f0_0000_0000, big * 3, big),
        0,
        "all three big-aligned must still let RM use a big page; a predicate that always \
         answers 4 KiB is not a fix, it is a different regression"
    );

    // Each quantity alone is sufficient to force the small page.
    for (at, offset, len, why) in [
        (big, 0x1000_u64, big, "offset"),
        (big + 0x1000, 0, big, "at"),
        (big, 0, big - 0x1000, "len"),
    ] {
        assert_eq!(
            nvos46_page_size_flag(at, offset, len),
            NVOS46_FLAGS_PAGE_SIZE_4KB,
            "a non-big-aligned {why} must force the small-page flag on its own"
        );
    }
}

/// ★★★★★ **w755 — ogkm's ACTUAL RULE, MODELLED, AND THE STORE-SLICE FLAG CHECKED AGAINST
/// IT.**
///
/// `[ogkm-580.159.04, virt_mem_allocator_gm107.c:726/:922/:1081/:1532 + virtual_mem.c:1323]`
/// RM does not require alignment — it requires **congruence**, and where the request is
/// page-aligned it *silently supplies* the physical side's page offset:
///
/// ```text
///   pageOffset = (reservation_base + offset) & (pageSize - 1)
///   vaLo       = ALIGN_DOWN(at, pageSize)
///   reject iff (at - vaLo) != 0 && (at - vaLo) != pageOffset
///   got        = vaLo + pageOffset
/// ```
///
/// ⊘ Modelled here rather than asserted as prose, so the claim the store-slice flag rests on
/// is executable. ⚠ The model is a MODEL: it reproduces the four lines above and nothing
/// else about RM. Its job is to show that `_4KB` makes `got == at` for every input, and that
/// a big page does not.
fn rm_would_return(at: u64, base: u64, offset: u64, page_size: u64) -> Option<u64> {
    let page_offset = (base.wrapping_add(offset)) & (page_size - 1);
    let va_lo = at & !(page_size - 1);
    let delta = at - va_lo;
    if delta != 0 && delta != page_offset {
        return None; // NV_ERR_INVALID_OFFSET
    }
    Some(va_lo + page_offset)
}

#[test]
fn the_store_slice_flag_is_the_only_one_that_survives_an_unknown_reservation_base() {
    let big = NVOS46_BIG_PAGE_BYTES;
    // A guest VA out of a page table, and a store offset out of the same walk — both are at
    // worst 4 KiB-granular, which is ALL we are entitled to assume.
    let at = 0x0000_00f0_0004_0000_u64;

    // ⊘ The reservation base is RM's choice and `reserve_gpga` never reports it, so the flag
    // must hold for EVERY base the driver could have picked. Sweep a representative set,
    // including bases that are big-aligned and bases that are only 4 KiB-aligned.
    let mut big_page_moved_it = 0;
    for base_k in [0_u64, 1, 3, 7, 15, 16, 17, 31, 64, 65] {
        let base = base_k * 0x1000;
        for off_k in [0_u64, 1, 2, 5, 15, 16, 48] {
            let offset = off_k * 0x1000;

            // ★ THE PROPERTY: at 4 KiB, the answer is the address we asked for. Always.
            assert_eq!(
                rm_would_return(at, base, offset, 0x1000),
                Some(at),
                "a 4 KiB page must honour the request exactly — base={base:#x} \
                 offset={offset:#x}. If this ever fails, the store-slice flag's whole \
                 argument is gone"
            );

            // ... and a big page does NOT, for most of them. Counted rather than asserted
            // per-case, because some (base, offset) pairs ARE congruent by luck and a
            // per-case assertion would be asserting the luck.
            if rm_would_return(at, base, offset, big) != Some(at) {
                big_page_moved_it += 1;
            }
        }
    }

    // ★ NON-VACUITY: the model must actually be able to produce the relocation this whole
    // finding is about. A model that never moves anything would satisfy the 4 KiB rows above
    // while demonstrating nothing.
    assert!(
        big_page_moved_it > 0,
        "the model never relocated anything at {big:#x}, so the 4 KiB rows prove nothing"
    );

    // ★★★ And the flag the store-slice path asks for is the one that holds.
    assert_eq!(
        kayfabe_abi::bringup::nvos46_page_size_flag_for_store_slice(),
        NVOS46_FLAGS_PAGE_SIZE_4KB,
        "the store-slice path must pin the small page table; congruence at any larger size \
         needs the reservation's GPGA, which `reserve_gpga` does not report"
    );
}

/// ★★★ **The row that names the SURPRISE**: a perfectly page-aligned request is *accepted*
/// and *moved*, rather than refused. That asymmetry is why constraint 28 has to compare
/// `dmaOffset` on the way out instead of trusting a status.
#[test]
fn a_page_aligned_request_is_accepted_and_silently_relocated() {
    let big = NVOS46_BIG_PAGE_BYTES;
    let at = 0x0000_00f0_0000_0000_u64; // big-aligned: intra-page offset 0
    let base = 0x1000; // reservation 4 KiB-granular, NOT big-aligned
    let got = rm_would_return(at, base, 0, big).expect("RM ACCEPTS this — that is the point");
    assert_ne!(
        got, at,
        "the request was page-aligned, accepted, and must come back MOVED — if it came back \
         equal there would be nothing for constraint 28 to catch"
    );
    assert_eq!(
        got - at,
        base,
        "moved by exactly the physical side's page offset"
    );

    // The third case: a request that names neither 0 nor the page offset is REFUSED outright.
    assert_eq!(
        rm_would_return(at + 0x2000, base, 0, big),
        None,
        "a VA whose intra-page offset is neither 0 nor the physical one must be refused, not \
         adjusted"
    );
}

/// ★★★ **w755 — THE PREDICTED PAGE SIZE IS THE ONE WE ASK FOR.**
///
/// ⊘⊘ Two constants for one decision is how a predicate and its subject drift apart, which is
/// this tree's most-repeated defect. The pre-flight assert computes with
/// `nvos46_store_slice_page_bytes()` and the ioctl carries
/// `nvos46_page_size_flag_for_store_slice()`; if those ever disagree the prediction is about
/// a request nobody makes, and it would be CONFIDENTLY WRONG rather than merely absent.
#[test]
fn the_predicted_page_size_is_the_one_we_ask_for() {
    use kayfabe_abi::bringup::{
        nvos46_page_size_flag_for_store_slice, nvos46_store_slice_page_bytes,
    };
    assert_eq!(
        nvos46_page_size_flag_for_store_slice(),
        NVOS46_FLAGS_PAGE_SIZE_4KB,
        "the store-slice flag moved"
    );
    assert_eq!(
        nvos46_store_slice_page_bytes(),
        4096,
        "the flag says 4 KiB and the byte count must say the same 4 KiB"
    );
}

/// ★★★★★ **w755 — THE PREDICTOR AGREES WITH THE MODEL OF ogkm, ON EVERY BRANCH.**
///
/// `rm_would_place` is the production predicate; `rm_would_return` above is the independent
/// transcription of ogkm's four lines written for the earlier test. ⊘ Checking one against
/// the other is a differential, not a tautology: they were written separately, and the
/// production one has to answer a three-way enum where the model answers `Option<u64>`.
#[test]
fn the_predictor_agrees_with_the_model_on_every_branch() {
    use kayfabe_abi::bringup::{PlacementPrediction, rm_would_place};
    let big = NVOS46_BIG_PAGE_BYTES;

    let mut seen_honoured = 0;
    let mut seen_relocate = 0;
    let mut seen_refuse = 0;

    for page in [0x1000_u64, big, 0x20_0000] {
        for at in [
            0_u64,
            0x1000,
            0x2000,
            big,
            big + 0x1000,
            0x20_0000,
            0x20_1000,
        ] {
            for phys in [0_u64, 0x1000, 0x3000, big, big + 0x2000, 0x20_0000 + 0x5000] {
                let predicted = rm_would_place(at, phys, page);
                let modelled = rm_would_return(at, phys, 0, page);
                match (predicted, modelled) {
                    (PlacementPrediction::Honoured, Some(g)) => {
                        assert_eq!(g, at, "Honoured must mean the model returns `at`");
                        seen_honoured += 1;
                    }
                    (PlacementPrediction::WouldRelocate { predicted: p }, Some(g)) => {
                        assert_eq!(p, g, "the predicted VA must be the modelled one");
                        assert_ne!(g, at, "a relocation that lands on `at` is not one");
                        seen_relocate += 1;
                    }
                    (PlacementPrediction::WouldRefuse, None) => seen_refuse += 1,
                    (p, m) => panic!(
                        "predictor and model disagree at at={at:#x} phys={phys:#x} \
                         page={page:#x}: {p:?} vs {m:?}"
                    ),
                }
            }
        }
    }

    // ★ NON-VACUITY, three ways. A predictor that answered one branch for everything would
    // agree with a model that did the same, and both could be wrong together.
    assert!(seen_honoured > 0, "no input was predicted Honoured");
    assert!(seen_relocate > 0, "no input was predicted WouldRelocate");
    assert!(seen_refuse > 0, "no input was predicted WouldRefuse");
}

/// ★★★ **The degenerate page size answers `WouldRefuse`, not garbage.**
///
/// ⊘ A `page_size` of 0 or a non-power-of-two would make `page_size - 1` a mask that means
/// nothing, and the prediction would be a confident number computed from nonsense. Refusing
/// is the only honest answer, and it is asserted rather than assumed.
#[test]
fn a_degenerate_page_size_is_refused_rather_than_computed_with() {
    use kayfabe_abi::bringup::{PlacementPrediction, rm_would_place};
    for bad in [0_u64, 3, 5, 6, 100, 0x1001] {
        assert_eq!(
            rm_would_place(0x1000, 0x1000, bad),
            PlacementPrediction::WouldRefuse,
            "page_size {bad:#x} is not a page size and must not be computed with"
        );
    }
    // ... and a good one still answers normally, so the guard is not swallowing everything.
    assert_eq!(
        rm_would_place(0x1000, 0x1000, 0x1000),
        PlacementPrediction::Honoured
    );
}

/// ★★★★★ **THE WHOLE POINT, in one row: at 4 KiB the pre-flight assert NEVER fires.**
///
/// ⊘ If it could fire at the size the store-slice path actually asks for, the fix would be
/// refusing real work rather than preventing a relocation. Swept over every (VA, phys) pair
/// a page-table walk can produce — both 4 KiB-granular, which is all we are entitled to
/// assume and all that is needed.
#[test]
fn at_four_kib_the_preflight_assert_can_never_refuse_a_walked_leaf() {
    use kayfabe_abi::bringup::{
        PlacementPrediction, nvos46_store_slice_page_bytes, rm_would_place,
    };
    let page = nvos46_store_slice_page_bytes();
    for at_k in 0..64_u64 {
        for phys_k in 0..64_u64 {
            assert_eq!(
                rm_would_place(at_k * 0x1000, phys_k * 0x1000, page),
                PlacementPrediction::Honoured,
                "at={:#x} phys={:#x} must be honoured at 4 KiB — if this ever fails the \
                 store-slice fix is refusing work instead of preventing a relocation",
                at_k * 0x1000,
                phys_k * 0x1000
            );
        }
    }
}
