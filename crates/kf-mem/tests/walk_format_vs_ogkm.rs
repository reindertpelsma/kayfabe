//! ★ The walk kernel's page-table descriptors (`kf_cuda::abi::kf_format_ver2/ver3`, pinned byte for
//! byte to `cuda/walk/kf_walk.cu` by `kf-cuda`'s descriptor differential) held to the ogkm-580
//! `dev_mmu.h` of every die group that walks them (`kf_chip::hwref`,
//! `docs/design/V3_HW_BOUNDARY_INVENTORY.md`). Until this test the two hand copies were only tied to
//! EACH OTHER; nothing tied either to the header.
//!
//! ⊘ Header facts only: bit positions, widths, shifts, aperture codes, entry sizes, PCF encodings.
//! The level GEOMETRY (which VA bits each level decodes, which levels are leaves) lives in RM's C
//! (`kern_gmmu_fmt_gp10x.c` / `_ga10x.c` / `_gh10x.c` / `_gb10x.c`), not in a header, and is not
//! checked here — the inventory lists it as a hand constant with its known per-die-group gaps.
//! It is here (kf-mem, which already depends on both crates) so `kf-cuda` keeps no dependency.

use kf_chip::hwref::DieGroup;
use kf_chip::hwref::expect::{range, val};
use kf_cuda::abi::{
    AP_INVALID, AP_PEER, AP_SYS, AP_SYS_NC, AP_VID, KfField, KfFormat, kf_format_ver2,
    kf_format_ver3,
};

const VER2: [DieGroup; 4] = [
    DieGroup::Tu10x,
    DieGroup::Ga100,
    DieGroup::Ga10x,
    DieGroup::Ad10x,
];
const VER3: [DieGroup; 3] = [DieGroup::Gh100, DieGroup::Gb10x, DieGroup::Gb20x];

/// A header `hi:lo` field as the descriptor's `(lo, bits)`, shifted down by `word` 64-bit words.
fn field(g: DieGroup, name: &str, word: u64) -> (u64, u64) {
    let (hi, lo) = range(g, name);
    (lo - 64 * word, hi - lo + 1)
}
fn ours(f: KfField) -> (u64, u64) {
    (u64::from(f.lo), u64::from(f.bits))
}
fn bit(i: u8) -> u64 {
    u64::from(i)
}

/// The aperture codes of `prefix` (`…_APERTURE_*`), in the descriptor's map order.
fn aperture_codes(g: DieGroup, prefix: &str) -> [u64; 4] {
    [
        "VIDEO_MEMORY",
        "PEER_MEMORY",
        "SYSTEM_COHERENT_MEMORY",
        "SYSTEM_NON_COHERENT_MEMORY",
    ]
    .map(|a| val(g, &format!("{prefix}_{a}")))
}

fn common(f: &KfFormat, g: DieGroup, v: &str) {
    let pte = format!("NV_MMU_{v}_PTE");
    assert_eq!(
        bit(f.valid_bit),
        range(g, &format!("{pte}_VALID")).1,
        "{g:?}"
    );
    let (ap_hi, ap_lo) = range(g, &format!("{pte}_APERTURE"));
    assert_eq!(
        (u64::from(f.ap_lo), u64::from(f.ap_bits)),
        (ap_lo, ap_hi - ap_lo + 1),
        "{g:?}"
    );
    // The PTE aperture code i maps to the report code in slot i: VID, PEER, SYS_COH, SYS_NC.
    assert_eq!(
        aperture_codes(g, &format!("{pte}_APERTURE")),
        [0, 1, 2, 3],
        "{g:?}"
    );
    assert_eq!(f.pte_ap_map, [AP_VID, AP_PEER, AP_SYS, AP_SYS_NC]);
    assert_eq!(val(g, &format!("{pte}__SIZE")), 8, "{g:?}");
    assert_eq!(u64::from(f.small_entry_bytes), 8);
    assert_eq!(u64::from(f.big_entry_bytes), 8);
    assert_eq!(
        u64::from(f.dir[4].entry_bytes),
        val(g, &format!("NV_MMU_{v}_DUAL_PDE__SIZE")),
        "{g:?}"
    );
    for d in &f.dir[..4] {
        if d.active != 0 {
            assert_eq!(
                u64::from(d.entry_bytes),
                val(g, &format!("NV_MMU_{v}_PDE__SIZE")),
                "{g:?}"
            );
        }
    }
    assert_eq!(ours(f.kind), field(g, &format!("{pte}_KIND"), 0), "{g:?}");
}

#[test]
fn the_ver2_descriptor_is_every_ver2_die_groups_dev_mmu_h() {
    let f = kf_format_ver2();
    for g in VER2 {
        common(&f, g, "VER2");
        // PTE address: VID 32:8 / SYS 53:8, both `<< ADDRESS_SHIFT` (12).
        assert_eq!(
            ours(f.addr_local),
            field(g, "NV_MMU_VER2_PTE_ADDRESS_VID", 0),
            "{g:?}"
        );
        assert_eq!(
            ours(f.addr_sys),
            field(g, "NV_MMU_VER2_PTE_ADDRESS_SYS", 0),
            "{g:?}"
        );
        assert_eq!(
            u64::from(f.addr_local.shift),
            val(g, "NV_MMU_VER2_PTE_ADDRESS_SHIFT"),
            "{g:?}"
        );
        assert_eq!(
            u64::from(f.addr_sys.shift),
            val(g, "NV_MMU_VER2_PTE_ADDRESS_SHIFT"),
            "{g:?}"
        );
        // The PDE reuses the same field spec: the header must agree it is the same field.
        assert_eq!(
            field(g, "NV_MMU_VER2_PDE_ADDRESS_VID", 0),
            ours(f.addr_local),
            "{g:?}"
        );
        assert_eq!(
            field(g, "NV_MMU_VER2_PDE_ADDRESS_SYS", 0),
            ours(f.addr_sys),
            "{g:?}"
        );
        assert_eq!(
            val(g, "NV_MMU_VER2_PDE_ADDRESS_SHIFT"),
            u64::from(f.addr_local.shift),
            "{g:?}"
        );
        assert_eq!(
            range(g, "NV_MMU_VER2_PDE_APERTURE"),
            range(g, "NV_MMU_VER2_PTE_APERTURE"),
            "{g:?}"
        );
        // PDE aperture codes: INVALID 0 / VID 1 / SYS_COH 2 / SYS_NC 3.
        let pde: Vec<u64> = [
            "INVALID",
            "VIDEO_MEMORY",
            "SYSTEM_COHERENT_MEMORY",
            "SYSTEM_NON_COHERENT_MEMORY",
        ]
        .iter()
        .map(|a| val(g, &format!("NV_MMU_VER2_PDE_APERTURE_{a}")))
        .collect();
        assert_eq!(pde, [0, 1, 2, 3], "{g:?}");
        assert_eq!(f.pde_ap_map, [AP_INVALID, AP_VID, AP_SYS, AP_SYS_NC]);
        assert_eq!(
            u64::from(f.pde_ap_invalid),
            val(g, "NV_MMU_VER2_PDE_APERTURE_INVALID"),
            "{g:?}"
        );
        // Dual PDE: big half in the low word (32:4 / 53:4, `<< 8`), small half in the high word
        // with the PDE's own layout (66:65 aperture, 96:72 / 117:72 address).
        assert_eq!(
            ours(f.big_addr_local),
            field(g, "NV_MMU_VER2_DUAL_PDE_ADDRESS_BIG_VID", 0),
            "{g:?}"
        );
        assert_eq!(
            ours(f.big_addr_sys),
            field(g, "NV_MMU_VER2_DUAL_PDE_ADDRESS_BIG_SYS", 0),
            "{g:?}"
        );
        assert_eq!(
            u64::from(f.big_addr_local.shift),
            val(g, "NV_MMU_VER2_DUAL_PDE_ADDRESS_BIG_SHIFT"),
            "{g:?}"
        );
        assert_eq!(
            range(g, "NV_MMU_VER2_DUAL_PDE_APERTURE_BIG"),
            range(g, "NV_MMU_VER2_PDE_APERTURE"),
            "{g:?}"
        );
        assert_eq!(
            field(g, "NV_MMU_VER2_DUAL_PDE_ADDRESS_SMALL_VID", 1),
            ours(f.addr_local),
            "{g:?}"
        );
        assert_eq!(
            field(g, "NV_MMU_VER2_DUAL_PDE_ADDRESS_SMALL_SYS", 1),
            ours(f.addr_sys),
            "{g:?}"
        );
        assert_eq!(
            field(g, "NV_MMU_VER2_DUAL_PDE_APERTURE_SMALL", 1),
            (u64::from(f.ap_lo), u64::from(f.ap_bits))
        );
        // Flag bits.
        for (ours, name) in [
            (f.bit_volatile, "NV_MMU_VER2_PTE_VOL"),
            (f.bit_privilege, "NV_MMU_VER2_PTE_PRIVILEGE"),
            (f.bit_read_only, "NV_MMU_VER2_PTE_READ_ONLY"),
            (f.bit_atomic_disable, "NV_MMU_VER2_PTE_ATOMIC_DISABLE"),
        ] {
            assert_eq!(range(g, name), (bit(ours), bit(ours)), "{g:?} {name}");
        }
        assert_eq!(
            range(g, "NV_MMU_VER2_PDE_VOL"),
            (bit(f.bit_volatile), bit(f.bit_volatile)),
            "{g:?}"
        );
    }
}

#[test]
fn the_ver3_descriptor_is_every_ver3_die_groups_dev_mmu_h() {
    let f = kf_format_ver3();
    for g in VER3 {
        common(&f, g, "VER3");
        // ONE address field for every aperture: 51:12, `<< 12`.
        assert_eq!(
            ours(f.addr_local),
            field(g, "NV_MMU_VER3_PTE_ADDRESS", 0),
            "{g:?}"
        );
        assert_eq!(
            ours(f.addr_sys),
            field(g, "NV_MMU_VER3_PTE_ADDRESS", 0),
            "{g:?}"
        );
        assert_eq!(
            u64::from(f.addr_local.shift),
            val(g, "NV_MMU_VER3_PTE_ADDRESS_SHIFT"),
            "{g:?}"
        );
        assert_eq!(
            field(g, "NV_MMU_VER3_PDE_ADDRESS", 0),
            ours(f.addr_local),
            "{g:?}"
        );
        assert_eq!(
            val(g, "NV_MMU_VER3_PDE_ADDRESS_SHIFT"),
            u64::from(f.addr_local.shift),
            "{g:?}"
        );
        assert_eq!(range(g, "NV_MMU_VER3_PDE_IS_PTE"), (0, 0), "{g:?}");
        // Dual PDE: big half 51:8 `<< 8` in the low word; small half = the PDE layout, high word.
        assert_eq!(
            ours(f.big_addr_local),
            field(g, "NV_MMU_VER3_DUAL_PDE_ADDRESS_BIG", 0),
            "{g:?}"
        );
        assert_eq!(
            u64::from(f.big_addr_local.shift),
            val(g, "NV_MMU_VER3_DUAL_PDE_ADDRESS_BIG_SHIFT"),
            "{g:?}"
        );
        assert_eq!(
            field(g, "NV_MMU_VER3_DUAL_PDE_ADDRESS_SMALL", 1),
            ours(f.addr_local),
            "{g:?}"
        );
        // PCF 7:3; SPARSE = 1; the four permission bits are the enumerants' low four bits.
        assert_eq!(ours(f.pcf), field(g, "NV_MMU_VER3_PTE_PCF", 0), "{g:?}");
        assert_eq!(
            u64::from(f.pcf_sparse),
            val(g, "NV_MMU_VER3_PTE_PCF_SPARSE"),
            "{g:?}"
        );
        let pcf_bit = |enumerant: &str| {
            let v = val(g, &format!("NV_MMU_VER3_PTE_PCF_{enumerant}"));
            assert!(v.is_power_of_two(), "{g:?} {enumerant}");
            u64::from(f.pcf.lo) + u64::from(v.trailing_zeros())
        };
        assert_eq!(
            bit(f.bit_volatile),
            pcf_bit("REGULAR_RW_ATOMIC_UNCACHED_ACE"),
            "{g:?}"
        );
        assert_eq!(
            bit(f.bit_privilege),
            pcf_bit("PRIVILEGE_RW_ATOMIC_CACHED_ACE"),
            "{g:?}"
        );
        assert_eq!(
            bit(f.bit_read_only),
            pcf_bit("REGULAR_RO_ATOMIC_CACHED_ACE"),
            "{g:?}"
        );
        assert_eq!(
            bit(f.bit_atomic_disable),
            pcf_bit("REGULAR_RW_NO_ATOMIC_CACHED_ACE"),
            "{g:?}"
        );
        // PDE apertures: 2:1, and code 0 is INVALID (the descriptor's "names no sub-level").
        assert_eq!(range(g, "NV_MMU_VER3_PDE_APERTURE"), (2, 1), "{g:?}");
        assert_eq!(val(g, "NV_MMU_VER3_PDE_APERTURE_INVALID"), 0, "{g:?}");
    }
}

/// ★ RATCHET — the VER3 PDE sparse encodings the walker does not honour (inventory, page-table
/// gaps): the PDE's PCF is `5:3` (not the PTE's `7:3`), and sparse is `1` **or** `3`. The walker's
/// sparse census reads the PTE field and matches `1` only. When that is fixed, this fails: update the
/// test and the inventory row.
#[test]
fn ratchet_the_ver3_pde_pcf_is_narrower_than_the_field_the_walker_reads() {
    let f = kf_format_ver3();
    for g in VER3 {
        assert_eq!(range(g, "NV_MMU_VER3_PDE_PCF"), (5, 3), "{g:?}");
        assert_ne!(
            field(g, "NV_MMU_VER3_PDE_PCF", 0),
            ours(f.pcf),
            "fixed? update the ratchet (inventory)"
        );
        assert_eq!(val(g, "NV_MMU_VER3_PDE_PCF_SPARSE_ATS_ALLOWED"), 1, "{g:?}");
        assert_eq!(
            val(g, "NV_MMU_VER3_PDE_PCF_SPARSE_ATS_NOT_ALLOWED"),
            3,
            "{g:?}: the sparse value the walker misses"
        );
    }
}
