//! ★ The guest's SPARSE leaf, told apart from an absent one (`docs/design/V3_SPARSE_PTE.md`, H6).
//!
//! On the GPU a sparse PTE absorbs an access and an invalid one faults, so the two must never be
//! the same state in the twin's pipeline. The encodings are held to the die groups' own
//! `dev_mmu.h` (`kf_chip::hwref`), not to a captured word.

use kf_chip::hwref::DieGroup;
use kf_chip::hwref::expect::{range, val};
use kf_cuda::abi::{kf_format_ver2, kf_format_ver3};
use kf_mem::sparse::{LeafState, classify_leaf};

const VER2: [DieGroup; 4] = [
    DieGroup::Tu10x,
    DieGroup::Ga100,
    DieGroup::Ga10x,
    DieGroup::Ad10x,
];
const VER3: [DieGroup; 3] = [DieGroup::Gh100, DieGroup::Gb10x, DieGroup::Gb20x];

/// A header `hi:lo` field with `value` in it.
fn put(g: DieGroup, name: &str, value: u64) -> u64 {
    value << range(g, name).1
}

/// VER2: the sparse PTE is VALID clear and VOLATILE set, at the header's own bit positions.
#[test]
fn ver2_sparse_is_valid_clear_volatile_set() {
    let f = kf_format_ver2();
    for g in VER2 {
        let raw = put(g, "NV_MMU_VER2_PTE_VOL", val(g, "NV_MMU_VER2_PTE_VOL_TRUE"))
            | put(
                g,
                "NV_MMU_VER2_PTE_VALID",
                val(g, "NV_MMU_VER2_PTE_VALID_FALSE"),
            );
        assert_eq!(classify_leaf(&f, raw), LeafState::Sparse, "{g:?} {raw:#x}");
    }
    // The fixture the walk kernel's own suite uses for the same encoding.
    assert_eq!(
        classify_leaf(&f, kf_cuda::synth::sparse_pte()),
        LeafState::Sparse
    );
    // The word run 5227 read at the faulting VA.
    assert_eq!(classify_leaf(&f, 0x8), LeafState::Sparse);
}

/// VER3: the sparse PTE is VALID clear and PCF == PCF_SPARSE, at the header's own field.
#[test]
fn ver3_sparse_is_valid_clear_pcf_sparse() {
    let f = kf_format_ver3();
    for g in VER3 {
        let raw = put(
            g,
            "NV_MMU_VER3_PTE_PCF",
            val(g, "NV_MMU_VER3_PTE_PCF_SPARSE"),
        );
        assert_eq!(classify_leaf(&f, raw), LeafState::Sparse, "{g:?} {raw:#x}");
    }
}

/// Ordinary invalid is unchanged: an all-zero word, and a word that only carries stale address
/// bits, are INVALID on both formats.
#[test]
fn an_absent_leaf_is_invalid_not_sparse() {
    for f in [kf_format_ver2(), kf_format_ver3()] {
        assert_eq!(classify_leaf(&f, 0), LeafState::Invalid);
        assert_eq!(classify_leaf(&f, 0x1234_5000_0000), LeafState::Invalid);
    }
}

/// VER2 words that look close to sparse but are not: the unmapped-big marker (PRIVILEGE only) and
/// NV4K (VALID clear, VOL and PRIVILEGE set, `kern_gmmu_gv100.c:66-74`).
#[test]
fn the_neighbours_of_sparse_are_invalid() {
    let f = kf_format_ver2();
    for g in VER2 {
        let vol = put(g, "NV_MMU_VER2_PTE_VOL", 1);
        let privilege = put(g, "NV_MMU_VER2_PTE_PRIVILEGE", 1);
        assert_eq!(
            classify_leaf(&f, privilege),
            LeafState::Invalid,
            "{g:?} unmapped big"
        );
        assert_eq!(
            classify_leaf(&f, vol | privilege),
            LeafState::Invalid,
            "{g:?} NV4K"
        );
    }
}

/// VER3 PCF values next to SPARSE that are not sparse: INVALID, MAPPING_NOWHERE, NO_VALID_4KB.
#[test]
fn ver3_neighbours_of_sparse_are_invalid() {
    let f = kf_format_ver3();
    for g in VER3 {
        for name in [
            "NV_MMU_VER3_PTE_PCF_INVALID",
            "NV_MMU_VER3_PTE_PCF_MAPPING_NOWHERE",
            "NV_MMU_VER3_PTE_PCF_NO_VALID_4KB_PAGE",
        ] {
            let raw = put(g, "NV_MMU_VER3_PTE_PCF", val(g, name));
            assert_eq!(classify_leaf(&f, raw), LeafState::Invalid, "{g:?} {name}");
        }
    }
}

/// A leaf with VALID set is a translation whatever else it carries, including the volatile bit
/// (a valid uncached mapping) and, on VER3, a PCF value equal to SPARSE.
#[test]
fn a_valid_leaf_is_never_sparse() {
    let f2 = kf_format_ver2();
    assert_eq!(classify_leaf(&f2, 0x1), LeafState::Valid);
    assert_eq!(
        classify_leaf(&f2, 0x9),
        LeafState::Valid,
        "valid + volatile"
    );
    assert_eq!(classify_leaf(&f2, 0x1234_5001), LeafState::Valid);
    let f3 = kf_format_ver3();
    assert_eq!(
        classify_leaf(&f3, 0x1 | (1 << 3)),
        LeafState::Valid,
        "valid with PCF==1"
    );
}
