//! The retired `ga10x_gmmu.rs` suite (14 tests, `kayfabe-chips-modules`), re-derived: every case is
//! a decode question with exactly one right answer in ogkm, and the file is organised by what a
//! wrong answer would do, because every wrong answer here is a plausible address, not an error.
//!
//! The two headline traps (`resume_from_fault.md` §6 hole 7, `#13`):
//!
//! 1. **`PD0`'s entry is sixteen bytes and names TWO sub-tables.** A decode that returns one
//!    drops a whole sub-tree with no diagnostic.
//! 2. **`PD1` is itself a leaf level on GA10x**, mapping 512 MiB. A design keyed on "leaves are
//!    PTEs" is wrong on this exact chip.
//!
//! The expectations are written out from the published header and the driver's level table,
//! NOT read back from `reference.rs`'s tables, so a wrong constant there cannot agree with
//! itself. Header lines (`H`, `LV`, `GA`, ...) are as in `reference.rs`.

use crate::reference::{Aperture, Decode, Edge, LevelShift, PageSize, Ver2};

// Level indices, in the driver's own numbering (`LV:58-104`).
const PD3: usize = 0;
const PD2: usize = 1;
const PD1: usize = 2;
const PD0: usize = 3;
const PT_BIG: usize = 4;
const PT_SMALL: usize = 5;

const VALID: u64 = 1;
/// `NV_MMU_VER2_*_VOL`, bit 3 (`H:124`).
const VOL: u64 = 1 << 3;
/// `NV_MMU_VER2_PTE_READ_ONLY`, bit 6 (`H:133`).
const RO: u64 = 1 << 6;

/// A single (non-dual) directory entry naming a vidmem sub-table: aperture 1 at 2:1, address
/// `>> 12` at 32:8.
fn pde_vid(next: u64) -> u128 {
    u128::from(((next >> 12) << 8) | (1 << 1))
}

/// A page-table entry mapping a vidmem page: valid, aperture 0, address `>> 12` at 32:8.
fn pte_vid(phys: u64) -> u128 {
    u128::from(((phys >> 12) << 8) | VALID)
}

fn fmt() -> Ver2 {
    Ver2::ga10x()
}

// =============================================================================================
// 1. The geometry -- a count, never a page size divided by an entry width
// =============================================================================================

/// Every level's `(shift, entries)` against the driver's own `virtAddrBitHi:Lo` pairs
/// (`LV:58-104`). The row that matters most is `PT_BIG`: **32** entries, not 512, and the only
/// way that stays true is if the count is read off the bit range.
#[test]
fn the_level_geometry_is_the_drivers_own_bit_ranges() {
    let f = fmt();
    let want = [
        (PD3, 47u8, 4u32),   // 48:47
        (PD2, 38, 512),      // 46:38
        (PD1, 29, 512),      // 37:29
        (PD0, 21, 256),      // 28:21
        (PT_BIG, 16, 32),    // 20:16  <- thirty-two
        (PT_SMALL, 12, 512), // 20:12
    ];
    for (level, shift, entries) in want {
        assert_eq!(
            f.level_shift(level),
            Some(LevelShift { shift, entries }),
            "level {level}"
        );
    }
    assert_eq!(
        f.small_page_walk_depth(),
        5,
        "a small-page walk is PD3->PD2->PD1->PD0->PT_SMALL"
    );
}

/// A level this format does not have is `None` and width 0 -- a loud refusal at the walker,
/// never a guessed stride -- and `Invalid`, not `Sparse`: sparse is a declaration the guest made,
/// and a level that does not exist carries none.
#[test]
fn a_level_this_format_does_not_have_is_a_refusal_and_not_a_default() {
    let f = fmt();
    for level in [6usize, 7, 200, 255] {
        assert_eq!(f.level_shift(level), None, "level {level}");
        assert_eq!(f.entry_size(level), 0, "level {level}");
        assert_eq!(
            f.decode_entry(level, u128::MAX),
            Decode::Invalid,
            "level {level}"
        );
    }
}

/// **Trap 1, half one.** `PD0`'s entry is sixteen bytes (`NV_MMU_VER2_DUAL_PDE__SIZE`, `H:112`);
/// every other level's is eight (`H:97, 157`).
#[test]
fn pd0s_entry_is_sixteen_bytes_and_every_other_levels_is_eight() {
    let f = fmt();
    assert_eq!(f.entry_size(PD0), 16);
    for level in [PD3, PD2, PD1, PT_BIG, PT_SMALL] {
        assert_eq!(f.entry_size(level), 8, "level {level}");
    }
}

// =============================================================================================
// 2. Trap 1 -- the DUAL entry, and both of its sub-tables
// =============================================================================================

/// **Both halves come back, and they are decoded with DIFFERENT shifts.**
///
/// `NV_MMU_VER2_DUAL_PDE_ADDRESS_BIG_SHIFT` is **8** and `_ADDRESS_SHIFT` is **12** (`H:104,
/// 111`). One shift for both puts every big-page table at one-sixteenth of its real address -- a
/// perfectly plausible framebuffer address.
#[test]
fn a_dual_pd0_entry_names_two_sub_tables_at_two_different_shifts() {
    let f = fmt();
    let small_tbl = 0x0012_3000u64;
    let big_tbl = 0x0045_6700u64; // 256-byte aligned, which is all the BIG half can name
    let lo = ((big_tbl >> 8) << 4) | (1 << 1); // BIG: 32:4 at shift 8, aperture 2:1
    let hi = ((small_tbl >> 12) << 8) | (1 << 1); // SMALL: bits 96:72 / 66:65 of the entry
    let raw = u128::from(lo) | (u128::from(hi) << 64);

    let Decode::Pde { edge, also } = f.decode_entry(PD0, raw) else {
        panic!("a dual entry with both apertures set is a Pde");
    };
    let also = also.expect("the SECOND sub-table must not be dropped (#13's shape, one level up)");
    assert_eq!(
        edge,
        Edge {
            next: small_tbl,
            aperture: Aperture::Vidmem,
            child_level: PT_SMALL
        },
        "edge is the SMALL-page sub-table"
    );
    assert_eq!(
        also.next, big_tbl,
        "also is the BIG-page sub-table, shift 8"
    );
    assert_eq!(also.child_level, PT_BIG);
}

/// Either half alone is enough, and the one that is present is the one that comes back.
#[test]
fn a_dual_entry_with_only_one_half_yields_only_that_half() {
    let f = fmt();
    let small_only = u128::from(((0x0033_4000u64 >> 12) << 8) | (1 << 1)) << 64;
    let Decode::Pde { edge, also } = f.decode_entry(PD0, small_only) else {
        panic!("small half present");
    };
    assert_eq!(edge.child_level, PT_SMALL);
    assert!(also.is_none());

    let big_only = u128::from(((0x0055_6600u64 >> 8) << 4) | (1 << 1));
    let Decode::Pde { edge, also } = f.decode_entry(PD0, big_only) else {
        panic!("big half present");
    };
    assert_eq!(
        edge.child_level, PT_BIG,
        "and it arrives as `edge`, not lost"
    );
    assert_eq!(edge.next, 0x0055_6600);
    assert!(also.is_none());
}

/// A `PD0` slot whose LOW half has the valid bit set is a **2 MiB page**, not a dual pointer:
/// `gmmuFmtEntryIsPte` asks the PTE's valid field first at a level that is both a page table and
/// a page directory (`IP:63-86`).
#[test]
fn a_pd0_slot_with_the_valid_bit_is_a_two_megabyte_leaf_and_not_a_dual_pointer() {
    let f = fmt();
    // Deliberately ALSO carrying a plausible small-half pointer in the high qword: the valid bit
    // must win, or a huge page would be walked as a directory.
    let raw = pte_vid(0x0080_0000) | (u128::from(((0x1234_5000u64 >> 12) << 8) | (1 << 1)) << 64);
    assert_eq!(
        f.decode_entry(PD0, raw),
        Decode::Leaf {
            phys: 0x0080_0000,
            aperture: Aperture::Vidmem,
            size: PageSize(2 << 20),
            read_only: false,
        }
    );
}

// =============================================================================================
// 3. Trap 2 -- PD1 is a leaf level on THIS chip
// =============================================================================================

/// `pLevels[2].bPageTable = NV_TRUE` is the whole GA10x delta (`GA:54-55`), and the gap that
/// silently dropped page-table writes for weeks. The same slot without the valid bit is still an
/// ordinary directory pointer, and its CHILD level is stated, never incremented by the caller.
#[test]
fn a_pd1_slot_with_the_valid_bit_is_a_512_mib_leaf() {
    let f = fmt();
    assert_eq!(
        f.decode_entry(PD1, pte_vid(0x2_0000_0000)),
        Decode::Leaf {
            phys: 0x2_0000_0000,
            aperture: Aperture::Vidmem,
            size: PageSize(512 << 20),
            read_only: false,
        },
        "on GA10x a PD1 entry can BE a 512 MiB page"
    );
    let Decode::Pde { edge, also } = f.decode_entry(PD1, pde_vid(0x0011_2000)) else {
        panic!("without VALID it is a directory");
    };
    assert_eq!(edge.next, 0x0011_2000);
    assert_eq!(edge.child_level, PD0);
    assert!(also.is_none());

    // And on the PRE-Ampere format the same bits at PD1 are NOT a leaf: the delta is exactly
    // the one statement, so the negative arm is asserted against `gp10x`.
    assert_eq!(
        Ver2::gp10x().decode_entry(PD1, pte_vid(0x2_0000_0000)),
        Decode::Invalid,
        "Turing's PD1 is a pure directory: the same bits read as a PDE whose aperture (bits 2:1 = 0) \
         is INVALID, never as a 512 MiB page"
    );
}

/// 512 MiB is **enumerated**, so a leaf claiming it decodes rather than becoming a loud
/// unknown-size fault. Leaving it out to dodge the whole-framebuffer alias would rebuild `#13`'s
/// silent drop at the other end; the alias is declined by policy, at the binding site.
#[test]
fn every_leaf_size_this_regime_can_express_is_enumerated_and_ascending() {
    let f = fmt();
    assert_eq!(
        f.page_sizes(),
        vec![
            PageSize(4 << 10),
            PageSize(64 << 10),
            PageSize(2 << 20),
            PageSize(512 << 20)
        ]
    );
    assert!(
        f.page_sizes().windows(2).all(|w| w[0].0 < w[1].0),
        "ascending is the contract"
    );
    assert_eq!(
        Ver2::gp10x().page_sizes(),
        vec![PageSize(4 << 10), PageSize(64 << 10), PageSize(2 << 20)],
        "Turing has no 512 MiB leaf"
    );
}

// =============================================================================================
// 4. The two aperture tables, which are NOT one table
// =============================================================================================

/// A PDE's aperture `0` is **INVALID** and a PTE's aperture `0` is **VIDEO MEMORY**
/// (`AP:165-201`, values `H:27-30` vs `H:80-83`). A shared decoder answers every leaf one
/// aperture out, and an aperture is half of every physical-address key.
#[test]
fn the_pde_aperture_table_and_the_pte_aperture_table_are_different_tables() {
    let f = fmt();
    // PDE side: 0 = INVALID, 1 = VIDEO, 2 = SYS_COH, 3 = SYS_NONCOH.
    for (bits, want) in [
        (1u64, Aperture::Vidmem),
        (2, Aperture::SysmemCoherent),
        (3, Aperture::SysmemNonCoherent),
    ] {
        let Decode::Pde { edge, .. } =
            f.decode_entry(PD2, u128::from(((0x9_0000u64 >> 12) << 8) | (bits << 1)))
        else {
            panic!("aperture {bits} is a live PDE");
        };
        assert_eq!(edge.aperture, want, "PDE aperture {bits}");
    }
    assert_eq!(
        f.decode_entry(PD2, u128::from((0x9_0000u64 >> 12) << 8)),
        Decode::Invalid,
        "a PDE aperture of ZERO is 'this sub-level is absent', not video memory"
    );

    // PTE side: 0 = VIDEO, 1 = PEER, 2 = SYS_COH, 3 = SYS_NONCOH.
    for (bits, want) in [
        (0u64, Aperture::Vidmem),
        (1, Aperture::Peer),
        (2, Aperture::SysmemCoherent),
        (3, Aperture::SysmemNonCoherent),
    ] {
        let Decode::Leaf { aperture, .. } =
            f.decode_entry(PT_SMALL, u128::from(VALID | (bits << 1) | (0x9u64 << 8)))
        else {
            panic!("a valid PTE with aperture {bits} is a leaf");
        };
        assert_eq!(aperture, want, "PTE aperture {bits}");
    }
}

/// The system-memory address field is **wider** than the video one: 46 bits (`53:8`, `H:139`)
/// against 25 (`(35-3):8`, `H:140`). The video mask for both would truncate every sysmem mapping
/// above 128 GiB into a plausible low address.
#[test]
fn the_system_memory_address_field_is_wider_than_the_video_one() {
    let f = fmt();
    let high = 0x0000_4000_0000_0000u64; // past what 25 bits << 12 can name
    let Decode::Leaf { phys, .. } =
        f.decode_entry(PT_SMALL, u128::from(VALID | (2 << 1) | ((high >> 12) << 8)))
    else {
        panic!("a sysmem PTE is a leaf");
    };
    assert_eq!(phys, high);
    // …and all 46 bits are read: the top of the field is entry bit 53.
    let all = (1u64 << 46) - 1;
    let Decode::Leaf { phys, .. } =
        f.decode_entry(PT_SMALL, u128::from(VALID | (2 << 1) | (all << 8)))
    else {
        panic!("a sysmem PTE is a leaf");
    };
    assert_eq!(phys, all << 12);
    // The VIDEO field stops at bit 32: a page number with its 26th bit set (entry bit 33, which
    // is not part of `(35-3):8`) decodes WITHOUT that bit.
    let pn = (1u64 << 25) | 0x5;
    let Decode::Leaf { phys, .. } = f.decode_entry(PT_SMALL, u128::from(VALID | (pn << 8))) else {
        panic!("a vidmem PTE is a leaf");
    };
    assert_eq!(
        phys,
        0x5 << 12,
        "the video address field is 25 bits, not 26"
    );
}

// =============================================================================================
// 5. SPARSE is a third state, at every level
// =============================================================================================

/// **Sparse is spelled "the target field says nothing, and VOLATILE is set"** (`SP:43-70`).
/// Without the volatile bit the same slot is `Invalid`. Folding sparse into a leaf binds a
/// mapping the guest declared backing-free; folding it into invalid loses the declaration.
#[test]
fn an_empty_slot_is_sparse_when_volatile_is_set_and_invalid_when_it_is_not() {
    let f = fmt();
    for level in [PD3, PD2, PD1, PD0, PT_BIG, PT_SMALL] {
        assert_eq!(
            f.decode_entry(level, 0),
            Decode::Invalid,
            "level {level}: nothing has been written here"
        );
        assert_eq!(
            f.decode_entry(level, u128::from(VOL)),
            Decode::Sparse,
            "level {level}: the guest DECLARED this range backing-free"
        );
    }
}

/// And the volatile bit must not turn a **live** entry into a sparse one: RM sets it on
/// perfectly ordinary mappings too, so it is only ever read where the entry is otherwise empty.
#[test]
fn the_volatile_bit_does_not_make_a_live_entry_sparse() {
    let f = fmt();
    assert!(matches!(
        f.decode_entry(PT_SMALL, pte_vid(0x7_000) | u128::from(VOL)),
        Decode::Leaf { .. }
    ));
    assert!(matches!(
        f.decode_entry(PD2, pde_vid(0x7_000) | u128::from(VOL)),
        Decode::Pde { .. }
    ));
}

/// The guest's read-only bit (`H:133`) is carried, so nothing downstream can widen rights it
/// never had -- and the negative arm, or the bit is being ignored.
#[test]
fn the_guests_read_only_bit_survives_the_decode() {
    let f = fmt();
    let Decode::Leaf { read_only, .. } = f.decode_entry(PT_BIG, pte_vid(0x1_0000) | u128::from(RO))
    else {
        panic!("a valid PTE is a leaf");
    };
    assert!(read_only);
    let Decode::Leaf { read_only, .. } = f.decode_entry(PT_BIG, pte_vid(0x1_0000)) else {
        panic!("a valid PTE is a leaf");
    };
    assert!(
        !read_only,
        "and the negative arm, or the bit is being ignored"
    );
}

/// The big-page table's leaf is 64 KiB and the small one's is 4 KiB: the two rows are
/// alternatives under one dual slot, not successive levels, and they map different sizes.
#[test]
fn the_two_leaf_tables_map_two_different_page_sizes() {
    let f = fmt();
    let Decode::Leaf { size, .. } = f.decode_entry(PT_BIG, pte_vid(0x10_0000)) else {
        panic!("leaf");
    };
    assert_eq!(size, PageSize(64 << 10));
    let Decode::Leaf { size, .. } = f.decode_entry(PT_SMALL, pte_vid(0x10_0000)) else {
        panic!("leaf");
    };
    assert_eq!(size, PageSize(4 << 10));
}
