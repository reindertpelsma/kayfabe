//! ★ The walk kernel's page-table descriptors (`kf_cuda::abi::kf_format_ver2/ver3`, pinned byte for
//! byte to `cuda/walk/kf_walk.cu` by `kf-cuda`'s descriptor differential) held to the ogkm-580
//! `dev_mmu.h` of every die group that walks them (`kf_chip::hwref`,
//! `docs/design/V3_HW_BOUNDARY_INVENTORY.md`). Until this test the two hand copies were only tied to
//! EACH OTHER; nothing tied either to the header.
//!
//! ★ TABLE-DRIVEN (2026-09-27, `STATUS_AND_HANDOFF.md` §4 item 6): each format is a table of rows,
//! one per NAMED `dev_mmu.h` define (`NV_MMU_{VER2,VER3}_…`, spelled after the prefix), each saying
//! which descriptor entry it pins — so what is checked reads as a list of header names, a new
//! descriptor field is one row, and a failure names the define. Every assertion of the
//! per-format functions this replaced is a row here.
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
/// The descriptor's aperture nibble, as `(lo, bits)`.
fn ap_nibble(f: &KfFormat) -> (u64, u64) {
    (u64::from(f.ap_lo), u64::from(f.ap_bits))
}

/// A descriptor bit-field, as `(lo, bits)`.
type Spec = fn(&KfFormat) -> (u64, u64);
/// A descriptor number: a bit position, a shift, an entry size, a code.
type Num = fn(&KfFormat) -> u64;
/// A descriptor aperture map (raw nibble → `KFWR_AP_*`).
type ApMap = fn(&KfFormat) -> [u8; 4];

/// ★ One row: a `dev_mmu.h` define, named after `NV_MMU_{VERn}_`, and what the descriptor must
/// say about it.
enum Row {
    /// A `hi:lo` field, `word` 64-bit words into the entry: the descriptor's `(lo, bits)`.
    Field(&'static str, u64, Spec),
    /// A one-bit `n:n` field: the descriptor's bit position.
    Bit(&'static str, Num),
    /// A plain value (`_SHIFT`, `__SIZE`, an enumerant): the descriptor's number.
    Val(&'static str, Num),
    /// A PCF enumerant that is ONE bit of the PCF field: the descriptor's bit position for it.
    PcfBit(&'static str, Num),
    /// An aperture enumeration `<name>_<APERTURE>`: the descriptor's map sends each code to the
    /// report aperture named beside it.
    Apertures(&'static str, ApMap, [(&'static str, u8); 4]),
    /// Two header fields that must be the same range (the descriptor holds one spec for both).
    Same(&'static str, &'static str),
    /// A header range the walker assumes, pinned as a literal `(hi, lo)`.
    Range(&'static str, (u64, u64)),
}

use Row::{Apertures, Bit, Field, PcfBit, Range, Same, Val};

const PTE_APERTURES: [(&str, u8); 4] = [
    ("VIDEO_MEMORY", AP_VID),
    ("PEER_MEMORY", AP_PEER),
    ("SYSTEM_COHERENT_MEMORY", AP_SYS),
    ("SYSTEM_NON_COHERENT_MEMORY", AP_SYS_NC),
];
const PDE_APERTURES: [(&str, u8); 4] = [
    ("INVALID", AP_INVALID),
    ("VIDEO_MEMORY", AP_VID),
    ("SYSTEM_COHERENT_MEMORY", AP_SYS),
    ("SYSTEM_NON_COHERENT_MEMORY", AP_SYS_NC),
];

/// Every row that holds in BOTH formats: validity, the aperture nibble and its codes, entry
/// sizes, and KIND.
fn common_rows() -> Vec<Row> {
    vec![
        Bit("PTE_VALID", |f| u64::from(f.valid_bit)),
        Field("PTE_APERTURE", 0, ap_nibble),
        // The PTE aperture code i maps to the report code in slot i: VID, PEER, SYS_COH, SYS_NC.
        Apertures("PTE_APERTURE", |f| f.pte_ap_map, PTE_APERTURES),
        Val("PTE__SIZE", |f| u64::from(f.small_entry_bytes)),
        Val("PTE__SIZE", |f| u64::from(f.big_entry_bytes)),
        Val("DUAL_PDE__SIZE", |f| u64::from(f.dir[4].entry_bytes)),
        Field("PTE_KIND", 0, |f| ours(f.kind)),
    ]
}

/// VER2 (Turing, Ampere, Ada): split vidmem/sysmem address fields; single-bit permissions.
fn ver2_rows() -> Vec<Row> {
    let mut rows = common_rows();
    rows.extend([
        // PTE address: VID 32:8 / SYS 53:8, both `<< ADDRESS_SHIFT` (12).
        Field("PTE_ADDRESS_VID", 0, |f| ours(f.addr_local)),
        Field("PTE_ADDRESS_SYS", 0, |f| ours(f.addr_sys)),
        Val("PTE_ADDRESS_SHIFT", |f| u64::from(f.addr_local.shift)),
        Val("PTE_ADDRESS_SHIFT", |f| u64::from(f.addr_sys.shift)),
        // The PDE reuses the same field spec: the header must agree it is the same field.
        Field("PDE_ADDRESS_VID", 0, |f| ours(f.addr_local)),
        Field("PDE_ADDRESS_SYS", 0, |f| ours(f.addr_sys)),
        Val("PDE_ADDRESS_SHIFT", |f| u64::from(f.addr_local.shift)),
        Same("PDE_APERTURE", "PTE_APERTURE"),
        // PDE aperture codes: INVALID 0 / VID 1 / SYS_COH 2 / SYS_NC 3.
        Apertures("PDE_APERTURE", |f| f.pde_ap_map, PDE_APERTURES),
        Val("PDE_APERTURE_INVALID", |f| u64::from(f.pde_ap_invalid)),
        // Dual PDE: big half in the low word (32:4 / 53:4, `<< 8`), small half in the high word
        // with the PDE's own layout (66:65 aperture, 96:72 / 117:72 address).
        Field("DUAL_PDE_ADDRESS_BIG_VID", 0, |f| ours(f.big_addr_local)),
        Field("DUAL_PDE_ADDRESS_BIG_SYS", 0, |f| ours(f.big_addr_sys)),
        Val("DUAL_PDE_ADDRESS_BIG_SHIFT", |f| {
            u64::from(f.big_addr_local.shift)
        }),
        Same("DUAL_PDE_APERTURE_BIG", "PDE_APERTURE"),
        Field("DUAL_PDE_ADDRESS_SMALL_VID", 1, |f| ours(f.addr_local)),
        Field("DUAL_PDE_ADDRESS_SMALL_SYS", 1, |f| ours(f.addr_sys)),
        Field("DUAL_PDE_APERTURE_SMALL", 1, ap_nibble),
        // Flag bits.
        Bit("PTE_VOL", |f| u64::from(f.bit_volatile)),
        Bit("PTE_PRIVILEGE", |f| u64::from(f.bit_privilege)),
        Bit("PTE_READ_ONLY", |f| u64::from(f.bit_read_only)),
        Bit("PTE_ATOMIC_DISABLE", |f| u64::from(f.bit_atomic_disable)),
        Bit("PDE_VOL", |f| u64::from(f.bit_volatile)),
    ]);
    rows
}

/// VER3 (Hopper, Blackwell): one address field for every aperture; permissions in the PCF.
fn ver3_rows() -> Vec<Row> {
    let mut rows = common_rows();
    rows.extend([
        // ONE address field for every aperture: 51:12, `<< 12`.
        Field("PTE_ADDRESS", 0, |f| ours(f.addr_local)),
        Field("PTE_ADDRESS", 0, |f| ours(f.addr_sys)),
        Val("PTE_ADDRESS_SHIFT", |f| u64::from(f.addr_local.shift)),
        Val("PTE_ADDRESS_SHIFT", |f| u64::from(f.addr_sys.shift)),
        Field("PDE_ADDRESS", 0, |f| ours(f.addr_local)),
        Val("PDE_ADDRESS_SHIFT", |f| u64::from(f.addr_local.shift)),
        Range("PDE_IS_PTE", (0, 0)),
        // Dual PDE: big half 51:8 `<< 8` in the low word; small half = the PDE layout, high word.
        Field("DUAL_PDE_ADDRESS_BIG", 0, |f| ours(f.big_addr_local)),
        Field("DUAL_PDE_ADDRESS_BIG", 0, |f| ours(f.big_addr_sys)),
        Val("DUAL_PDE_ADDRESS_BIG_SHIFT", |f| {
            u64::from(f.big_addr_local.shift)
        }),
        Field("DUAL_PDE_ADDRESS_SMALL", 1, |f| ours(f.addr_local)),
        // PCF 7:3; SPARSE = 1; the four permission bits are the enumerants' low four bits.
        Field("PTE_PCF", 0, |f| ours(f.pcf)),
        Val("PTE_PCF_SPARSE", |f| u64::from(f.pcf_sparse)),
        PcfBit("PTE_PCF_REGULAR_RW_ATOMIC_UNCACHED_ACE", |f| {
            u64::from(f.bit_volatile)
        }),
        PcfBit("PTE_PCF_PRIVILEGE_RW_ATOMIC_CACHED_ACE", |f| {
            u64::from(f.bit_privilege)
        }),
        PcfBit("PTE_PCF_REGULAR_RO_ATOMIC_CACHED_ACE", |f| {
            u64::from(f.bit_read_only)
        }),
        PcfBit("PTE_PCF_REGULAR_RW_NO_ATOMIC_CACHED_ACE", |f| {
            u64::from(f.bit_atomic_disable)
        }),
        // PDE apertures: 2:1, and code 0 is INVALID (the descriptor's "names no sub-level").
        Range("PDE_APERTURE", (2, 1)),
        Apertures("PDE_APERTURE", |f| f.pde_ap_map, PDE_APERTURES),
        Val("PDE_APERTURE_INVALID", |f| u64::from(f.pde_ap_invalid)),
    ]);
    rows
}

/// Hold `f` to every row, for every die group `groups` names, with header names under
/// `NV_MMU_{v}_`.
fn check(f: &KfFormat, v: &str, groups: &[DieGroup], rows: &[Row]) {
    let n = |s: &str| format!("NV_MMU_{v}_{s}");
    for &g in groups {
        for row in rows {
            match *row {
                Field(name, word, spec) => {
                    assert_eq!(
                        spec(f),
                        field(g, &n(name), word),
                        "{g:?} {} (lo, bits), word {word}",
                        n(name)
                    );
                }
                Bit(name, pos) => {
                    assert_eq!(range(g, &n(name)), (pos(f), pos(f)), "{g:?} {}", n(name));
                }
                Val(name, num) => assert_eq!(num(f), val(g, &n(name)), "{g:?} {}", n(name)),
                PcfBit(name, pos) => {
                    let e = val(g, &n(name));
                    assert!(
                        e.is_power_of_two(),
                        "{g:?} {}: not one bit of the PCF",
                        n(name)
                    );
                    assert_eq!(
                        pos(f),
                        u64::from(f.pcf.lo) + u64::from(e.trailing_zeros()),
                        "{g:?} {}",
                        n(name)
                    );
                }
                Apertures(name, map, names) => {
                    let map = map(f);
                    for (a, ap) in names {
                        let code = val(g, &n(&format!("{name}_{a}")));
                        let slot = usize::try_from(code).ok().filter(|&c| c < map.len());
                        assert_eq!(
                            slot.map(|c| map[c]),
                            Some(ap),
                            "{g:?} {}_{a} = {code}",
                            n(name)
                        );
                    }
                }
                Same(a, b) => assert_eq!(
                    range(g, &n(a)),
                    range(g, &n(b)),
                    "{g:?} {} vs {}",
                    n(a),
                    n(b)
                ),
                Range(name, want) => assert_eq!(range(g, &n(name)), want, "{g:?} {}", n(name)),
            }
        }
        // Every ACTIVE directory level uses the PDE's entry size (level 4 is the dual level).
        for d in f.dir[..4].iter().filter(|d| d.active != 0) {
            assert_eq!(u64::from(d.entry_bytes), val(g, &n("PDE__SIZE")), "{g:?}");
        }
    }
}

#[test]
fn the_ver2_descriptor_is_every_ver2_die_groups_dev_mmu_h() {
    let f = kf_format_ver2();
    // ⊘ The two maps as the descriptor states them, beside the per-code rows that derive them.
    assert_eq!(f.pte_ap_map, [AP_VID, AP_PEER, AP_SYS, AP_SYS_NC]);
    assert_eq!(f.pde_ap_map, [AP_INVALID, AP_VID, AP_SYS, AP_SYS_NC]);
    check(&f, "VER2", &VER2, &ver2_rows());
}

#[test]
fn the_ver3_descriptor_is_every_ver3_die_groups_dev_mmu_h() {
    let f = kf_format_ver3();
    assert_eq!(f.pte_ap_map, [AP_VID, AP_PEER, AP_SYS, AP_SYS_NC]);
    check(&f, "VER3", &VER3, &ver3_rows());
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
