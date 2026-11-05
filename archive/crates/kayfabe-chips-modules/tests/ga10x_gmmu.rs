//! ★★★ **The GA10x page-table format, exercised against the two traps that cost weeks.**
//!
//! `#149`. Every case below is a decode question with exactly one right answer in
//! `ogkm-580`'s own headers, and the file is organised by *what a wrong answer would do*
//! rather than by method name — because every wrong answer here is a plausible address,
//! not an error.
//!
//! The two headline traps (`resume_from_fault.md` §6 hole 7, `#13`):
//!
//! 1. **`PD0`'s entry is sixteen bytes and names TWO sub-tables.** A decode that returns
//!    one drops a whole sub-tree with no diagnostic.
//! 2. **`PD1` is itself a leaf level on this generation**, mapping 512 MiB. A design keyed
//!    on *"leaves are PTEs"* is wrong on this exact chip.
//!
//! And the quieter one that is just as expensive: **the PDE aperture table and the PTE
//! aperture table are different tables** — a PDE's `1` is video memory and a PTE's `0` is
//! (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/arch/maxwell/kern_gmmu_fmt_gm10x.c:165-201`).
//! A decoder that shared one function between them puts every leaf one aperture out, and
//! an aperture is half of every physical-address key in this port.

use kayfabe_arch::{Aperture, GmmuFmt, GmmuVersion, LevelShift, PageSize, PteDecode};
use kayfabe_chips::Ga10xGmmu;

/// Level indices, in the format's own numbering (`kern_gmmu_fmt_gp10x.c:48-105`).
const PD3: u8 = 0;
const PD2: u8 = 1;
const PD1: u8 = 2;
const PD0: u8 = 3;
const PT_BIG: u8 = 4;
const PT_SMALL: u8 = 5;

/// `NV_MMU_VER2_PTE_VALID`.
const VALID: u64 = 1;
/// `NV_MMU_VER2_*_VOL`, bit 3 — sparse when the valid/aperture field says "nothing here".
const VOL: u64 = 1 << 3;
/// `NV_MMU_VER2_PTE_READ_ONLY`, bit 6.
const RO: u64 = 1 << 6;

/// A single (non-dual) page-directory entry pointing at a vidmem sub-table.
fn pde_vid(next: u64) -> u128 {
    u128::from(((next >> 12) << 8) | (1 << 1))
}

/// A page-table entry mapping a vidmem page.
fn pte_vid(phys: u64) -> u128 {
    u128::from(((phys >> 12) << 8) | VALID)
}

// =====================================================================================
// 1. The geometry — a count, never a page size divided by an entry width
// =====================================================================================

/// ★★ Every level's `(shift, entries)` against RM's own `virtAddrBitHi:Lo` pairs.
///
/// ⊘ The row that matters most is `PT_BIG`: **32** entries, not 512. `LevelShift`'s own
/// docs name the failure — `page_bytes / entry_size` over-reads a big-page table by
/// 3 840 bytes — and the only way that stays true is if the count is read off the bit
/// range rather than derived.
#[test]
fn the_level_geometry_is_the_drivers_own_bit_ranges() {
    let f = Ga10xGmmu::new();
    let want = [
        (PD3, 47u8, 4u32),   // 48:47
        (PD2, 38, 512),      // 46:38
        (PD1, 29, 512),      // 37:29
        (PD0, 21, 256),      // 28:21
        (PT_BIG, 16, 32),    // 20:16  ★ thirty-two
        (PT_SMALL, 12, 512), // 20:12
    ];
    for (level, shift, entries) in want {
        assert_eq!(
            f.level_shift(level),
            Some(LevelShift { shift, entries }),
            "level {level}"
        );
    }
    assert_eq!(f.version(), GmmuVersion::Ver2);
    assert_eq!(
        f.levels(),
        5,
        "a small-page walk is PD3→PD2→PD1→PD0→PT_SMALL"
    );
}

/// ⊘ A level this format does not have is **`None`** and width **0** — a loud refusal at
/// the walker, never a guessed stride.
#[test]
fn a_level_this_format_does_not_have_is_a_refusal_and_not_a_default() {
    let f = Ga10xGmmu::new();
    for level in [6u8, 7, 200, 255] {
        assert_eq!(f.level_shift(level), None, "level {level}");
        assert_eq!(f.entry_size(level), 0, "level {level}");
        assert_eq!(
            f.decode_entry(level, u128::MAX),
            PteDecode::Invalid,
            "★ Invalid, not Sparse: sparse is a declaration the guest made, and a level \
             that does not exist carries none"
        );
    }
}

/// ★★★ **Trap 1, half one.** `PD0`'s entry is sixteen bytes; every other level's is eight.
#[test]
fn pd0s_entry_is_sixteen_bytes_and_every_other_levels_is_eight() {
    let f = Ga10xGmmu::new();
    assert_eq!(f.entry_size(PD0), 16, "NV_MMU_VER2_DUAL_PDE__SIZE");
    for level in [PD3, PD2, PD1, PT_BIG, PT_SMALL] {
        assert_eq!(f.entry_size(level), 8, "level {level}");
    }
}

// =====================================================================================
// 2. Trap 1 — the DUAL entry, and both of its sub-tables
// =====================================================================================

/// ★★★ **Both halves come back, and they are decoded with DIFFERENT shifts.**
///
/// `NV_MMU_VER2_DUAL_PDE_ADDRESS_BIG_SHIFT` is **8** and `_ADDRESS_SHIFT` is **12**
/// (`ogkm-580: dev_mmu.h:104, 111`). Using one shift for both puts every big-page table at
/// one-sixteenth of its real address — a perfectly plausible framebuffer address.
#[test]
fn a_dual_pd0_entry_names_two_sub_tables_at_two_different_shifts() {
    let f = Ga10xGmmu::new();
    let small_tbl = 0x0012_3000u64;
    let big_tbl = 0x0045_6700u64; // 256-byte aligned, which is all the BIG half can name
    let lo = ((big_tbl >> 8) << 4) | (1 << 1);
    let hi = ((small_tbl >> 12) << 8) | (1 << 1);
    let raw = u128::from(lo) | (u128::from(hi) << 64);

    let PteDecode::Pde { edge, also } = f.decode_entry(PD0, raw) else {
        panic!("a dual entry with both apertures set is a Pde");
    };
    let also = also.expect(
        "★★★ the SECOND sub-table must not be dropped — that is #13's \
                            shape one level up the tree",
    );
    assert_eq!(edge.next, small_tbl, "edge is the SMALL-page sub-table");
    assert_eq!(edge.child_level, PT_SMALL);
    assert_eq!(edge.aperture, Aperture::Vidmem);
    assert_eq!(
        also.next, big_tbl,
        "also is the BIG-page sub-table, shift 8"
    );
    assert_eq!(also.child_level, PT_BIG);
}

/// ★ Either half alone is enough, and the one that is present is the one that comes back.
#[test]
fn a_dual_entry_with_only_one_half_yields_only_that_half() {
    let f = Ga10xGmmu::new();
    let small_only = u128::from(((0x0033_4000u64 >> 12) << 8) | (1 << 1)) << 64;
    let PteDecode::Pde { edge, also } = f.decode_entry(PD0, small_only) else {
        panic!("small half present");
    };
    assert_eq!(edge.child_level, PT_SMALL);
    assert!(also.is_none());

    let big_only = u128::from(((0x0055_6600u64 >> 8) << 4) | (1 << 1));
    let PteDecode::Pde { edge, also } = f.decode_entry(PD0, big_only) else {
        panic!("big half present");
    };
    assert_eq!(
        edge.child_level, PT_BIG,
        "★ and it arrives as `edge`, not lost"
    );
    assert_eq!(edge.next, 0x0055_6600);
    assert!(also.is_none());
}

/// ★★ A `PD0` slot whose LOW half has the valid bit set is a **2 MiB page**, not a dual
/// pointer — `gmmuFmtEntryIsPte` asks the PTE's valid field first at a level that is both
/// a page table and a page directory (`ogkm-580: gmmu_fmt.c:71-76`).
#[test]
fn a_pd0_slot_with_the_valid_bit_is_a_two_megabyte_leaf_and_not_a_dual_pointer() {
    let f = Ga10xGmmu::new();
    // Deliberately ALSO carrying a plausible small-half pointer in the high qword: the
    // valid bit must win, or a huge page would be walked as a directory.
    let raw = pte_vid(0x0080_0000) | (u128::from(((0x1234_5000u64 >> 12) << 8) | (1 << 1)) << 64);
    assert_eq!(
        f.decode_entry(PD0, raw),
        PteDecode::Leaf {
            phys: 0x0080_0000,
            aperture: Aperture::Vidmem,
            size: PageSize(2 << 20),
            read_only: false,
        }
    );
}

// =====================================================================================
// 3. Trap 2 — PD1 is a leaf level on THIS chip
// =====================================================================================

/// ★★★ **`pLevels[2].bPageTable = NV_TRUE` — the whole GA10x delta**
/// (`ogkm-580: kern_gmmu_fmt_ga10x.c:52`), and the gap that silently dropped page-table
/// writes for weeks.
#[test]
fn a_pd1_slot_with_the_valid_bit_is_a_512_mib_leaf() {
    let f = Ga10xGmmu::new();
    assert_eq!(
        f.decode_entry(PD1, pte_vid(0x2_0000_0000)),
        PteDecode::Leaf {
            phys: 0x2_0000_0000,
            aperture: Aperture::Vidmem,
            size: PageSize(512 << 20),
            read_only: false,
        },
        "★★★ on GA10x a PD1 entry can BE a 512 MiB page"
    );
    // …and without the valid bit it is still an ordinary directory pointer.
    let PteDecode::Pde { edge, also } = f.decode_entry(PD1, pde_vid(0x0011_2000)) else {
        panic!("without VALID it is a directory");
    };
    assert_eq!(edge.next, 0x0011_2000);
    assert_eq!(
        edge.child_level, PD0,
        "★ the CHILD level is stated, never incremented"
    );
    assert!(also.is_none());
}

/// ★ 512 MiB is **enumerated**, so a leaf claiming it decodes rather than becoming a loud
/// `UnknownLeafSize`. ⊘ Leaving it out to dodge the whole-framebuffer alias would rebuild
/// `#13`'s silent drop at the other end; the alias is declined by *policy*, at the binding
/// site, and not by pretending the size does not exist.
#[test]
fn every_leaf_size_this_regime_can_express_is_enumerated_and_ascending() {
    let f = Ga10xGmmu::new();
    assert_eq!(
        f.page_sizes(),
        &[
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
}

// =====================================================================================
// 4. The two aperture tables, which are NOT one table
// =====================================================================================

/// ★★★ A PDE's aperture `0` is **INVALID** and a PTE's aperture `0` is **VIDEO MEMORY**.
///
/// The single easiest thing to get wrong on this regime, and the failure is silent: a
/// shared decoder answers every leaf one aperture out, and `Aperture` is half of every
/// physical-address key this port has.
#[test]
fn the_pde_aperture_table_and_the_pte_aperture_table_are_different_tables() {
    let f = Ga10xGmmu::new();
    // PDE side: 0 = INVALID, 1 = VIDEO, 2 = SYS_COH, 3 = SYS_NONCOH.
    for (bits, want) in [
        (1u64, Aperture::Vidmem),
        (2, Aperture::SysmemCoherent),
        (3, Aperture::SysmemNonCoherent),
    ] {
        let PteDecode::Pde { edge, .. } =
            f.decode_entry(PD2, u128::from(((0x9_0000u64 >> 12) << 8) | (bits << 1)))
        else {
            panic!("aperture {bits} is a live PDE");
        };
        assert_eq!(edge.aperture, want, "PDE aperture {bits}");
    }
    assert_eq!(
        f.decode_entry(PD2, u128::from((0x9_0000u64 >> 12) << 8)),
        PteDecode::Invalid,
        "★ a PDE aperture of ZERO is 'this sub-level is absent', not video memory"
    );

    // PTE side: 0 = VIDEO, 1 = PEER, 2 = SYS_COH, 3 = SYS_NONCOH.
    for (bits, want) in [
        (0u64, Aperture::Vidmem),
        (1, Aperture::Peer),
        (2, Aperture::SysmemCoherent),
        (3, Aperture::SysmemNonCoherent),
    ] {
        let PteDecode::Leaf { aperture, .. } =
            f.decode_entry(PT_SMALL, u128::from(VALID | (bits << 1) | ((0x9u64) << 8)))
        else {
            panic!("a valid PTE with aperture {bits} is a leaf");
        };
        assert_eq!(aperture, want, "PTE aperture {bits}");
    }
}

/// ★★ The system-memory address field is **wider** than the video one — 46 bits against 25
/// (`dev_mmu.h:113, 119, 139, 140`). A decoder that used the video mask for both would
/// truncate every sysmem mapping above 128 GiB into a plausible low address.
#[test]
fn the_system_memory_address_field_is_wider_than_the_video_one() {
    let f = Ga10xGmmu::new();
    let high = 0x0000_4000_0000_0000u64; // past what 25 bits << 12 can name
    let PteDecode::Leaf { phys, .. } =
        f.decode_entry(PT_SMALL, u128::from(VALID | (2 << 1) | ((high >> 12) << 8)))
    else {
        panic!("a sysmem PTE is a leaf");
    };
    assert_eq!(phys, high);
}

// =====================================================================================
// 5. SPARSE is a third state, at every level
// =====================================================================================

/// ★★★ **Sparse is spelled "the target field says nothing, and VOLATILE is set"**
/// (`ogkm-580: kern_gmmu_gm200.c:46-70`). Without the volatile bit the same slot is
/// `Invalid`.
///
/// Conflating them is `reachability_on_transition.md` §3.6 hole 6: fold sparse into a leaf
/// and a valid→sparse transition **binds** a mapping the guest declared backing-free; fold
/// it into invalid and the declaration disappears.
#[test]
fn an_empty_slot_is_sparse_when_volatile_is_set_and_invalid_when_it_is_not() {
    let f = Ga10xGmmu::new();
    for level in [PD3, PD2, PD1, PD0, PT_BIG, PT_SMALL] {
        assert_eq!(
            f.decode_entry(level, 0),
            PteDecode::Invalid,
            "level {level}: nothing has been written here"
        );
        assert_eq!(
            f.decode_entry(level, u128::from(VOL)),
            PteDecode::Sparse,
            "level {level}: the guest DECLARED this range backing-free"
        );
    }
}

/// ⊘ And the volatile bit must not turn a **live** entry into a sparse one: RM sets it on
/// perfectly ordinary mappings too (`NV_MMU_VER2_PTE_VOL`), so it is only ever read where
/// the entry is otherwise empty.
#[test]
fn the_volatile_bit_does_not_make_a_live_entry_sparse() {
    let f = Ga10xGmmu::new();
    assert!(matches!(
        f.decode_entry(PT_SMALL, pte_vid(0x7_000) | u128::from(VOL)),
        PteDecode::Leaf { .. }
    ));
    assert!(matches!(
        f.decode_entry(PD2, pde_vid(0x7_000) | u128::from(VOL)),
        PteDecode::Pde { .. }
    ));
}

/// ★ The guest's read-only bit is carried, so nothing downstream can widen rights it never
/// had.
#[test]
fn the_guests_read_only_bit_survives_the_decode() {
    let f = Ga10xGmmu::new();
    let PteDecode::Leaf { read_only, .. } =
        f.decode_entry(PT_BIG, pte_vid(0x1_0000) | u128::from(RO))
    else {
        panic!("a valid PTE is a leaf");
    };
    assert!(read_only);
    let PteDecode::Leaf { read_only, .. } = f.decode_entry(PT_BIG, pte_vid(0x1_0000)) else {
        panic!("a valid PTE is a leaf");
    };
    assert!(
        !read_only,
        "and the negative arm, or the bit is being ignored"
    );
}

/// ★★ The big-page table's leaf is 64 KiB and the small one's is 4 KiB — the two rows are
/// alternatives under one dual slot, not successive levels, and they map different sizes.
#[test]
fn the_two_leaf_tables_map_two_different_page_sizes() {
    let f = Ga10xGmmu::new();
    let PteDecode::Leaf { size, .. } = f.decode_entry(PT_BIG, pte_vid(0x10_0000)) else {
        panic!("leaf");
    };
    assert_eq!(size, PageSize(64 << 10));
    let PteDecode::Leaf { size, .. } = f.decode_entry(PT_SMALL, pte_vid(0x10_0000)) else {
        panic!("leaf");
    };
    assert_eq!(size, PageSize(4 << 10));
}
