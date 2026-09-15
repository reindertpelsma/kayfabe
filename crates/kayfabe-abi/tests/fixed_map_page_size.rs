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
        let flag = nvos46_page_size_flag(va, 0x1000);
        assert_eq!(
            flag, NVOS46_FLAGS_PAGE_SIZE_4KB,
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
        nvos46_page_size_flag(0x0000_00f0_0000_0000, NVOS46_BIG_PAGE_BYTES),
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
        nvos46_page_size_flag(0x0000_00f0_0000_0000, 0x8600),
        NVOS46_FLAGS_PAGE_SIZE_4KB,
        "★ Length is half the predicate. A 0x8600-byte range at a 64 KiB boundary is not a \
         whole number of big pages, so a big PTE would cover bytes the caller did not ask \
         for."
    );
}

#[test]
fn the_predicate_is_exactly_big_page_alignment_of_both_terms() {
    // ★ NON-VACUITY: the boundary is where it is claimed to be, on both axes, and the
    // function is not simply "always 4KB" (which would pass the first three tests).
    assert_eq!(nvos46_page_size_flag(NVOS46_BIG_PAGE_BYTES, NVOS46_BIG_PAGE_BYTES), 0);
    assert_eq!(
        nvos46_page_size_flag(NVOS46_BIG_PAGE_BYTES - 0x1000, NVOS46_BIG_PAGE_BYTES),
        NVOS46_FLAGS_PAGE_SIZE_4KB
    );
    assert_eq!(
        nvos46_page_size_flag(NVOS46_BIG_PAGE_BYTES, NVOS46_BIG_PAGE_BYTES - 0x1000),
        NVOS46_FLAGS_PAGE_SIZE_4KB
    );
    assert_eq!(
        NVOS46_BIG_PAGE_BYTES, 65_536,
        "★ NON-VACUITY: the family big-page size moved. Every row above is stated in terms \
         of it, and a different value means the w744 rows are being checked against a \
         boundary hardware does not have."
    );
}
