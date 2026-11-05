//! The two VER2 artifacts `kf-cuda` ships or builds tests with, each judged by the ogkm-derived
//! reference instead of by itself:
//!
//! * [`kf_format_ver2`], the descriptor handed to the walk kernel at launch. It is a hand
//!   transcription of the `.cu`'s own `kf_format_ver2()`, and `walk_abi_matches_the_cu.rs` pins it
//!   to that function byte for byte; this file pins it to the DRIVER's level table and header,
//!   which is the other half of "derive, never capture".
//! * [`kf_cuda::synth`], the independent table builder the isolate and the gates point the kernel
//!   at. Its docs promise it shares no code with either decoder; here its encoders are decoded by
//!   one.

use kf_cuda::abi::{
    AP_INVALID, AP_PEER, AP_SYS, AP_SYS_NC, AP_VID, KF_DIRS, KF_PS_NONE, KF_TBL_VER2, KfField,
    PS_2M, PS_4K, PS_64K, PS_512M, kf_format_ver2,
};
use kf_cuda::synth;

use crate::reference::{
    Aperture, Bits, Decode, Image, Page, PageSize, Ver2, ZeroChild, decode_subtree, hdr,
    pde_aperture, pte_aperture,
};

fn field(bits: Bits, shift: u32) -> KfField {
    KfField {
        lo: u8::try_from(bits.lo).unwrap(),
        bits: u8::try_from(bits.width()).unwrap(),
        shift: u8::try_from(shift).unwrap(),
        pad: 0,
    }
}

/// The page-size CODE the descriptor and the report use for a leaf size.
fn ps_code(size: PageSize) -> u8 {
    match size.0 {
        s if s == 4 << 10 => PS_4K,
        s if s == 64 << 10 => PS_64K,
        s if s == 2 << 20 => PS_2M,
        s if s == 512 << 20 => PS_512M,
        s => panic!("no page-size code for {s:#x}"),
    }
}

/// The directory half of the descriptor against the driver's level rows: five slots, the first
/// inactive (VER3's PD4), then PD3..PD0 with the geometry the bit ranges give and a leaf code on
/// exactly the levels the driver marks `bPageTable`.
#[test]
fn the_shipped_ver2_directory_geometry_is_the_drivers_level_table() {
    let d = kf_format_ver2();
    let fmt = Ver2::ga10x();
    assert_eq!(d.table_version, KF_TBL_VER2);
    assert_eq!(
        d.first_dir as usize,
        KF_DIRS - 4,
        "PD3 is the root: four directory levels"
    );
    assert_eq!(
        d.dir[0].active, 0,
        "PD4 is VER3's; VER2 leaves the slot a pass-through"
    );

    for level in 0..4 {
        let slot = d.dir[d.first_dir as usize + level];
        let row = fmt.levels()[level];
        let geo = fmt.level_shift(level).unwrap();
        assert_eq!(slot.active, 1, "level {level}");
        assert_eq!(u32::from(slot.va_lo), row.va_lo, "level {level} va_lo");
        assert_eq!(
            u32::from(slot.entries),
            geo.entries,
            "level {level} entries"
        );
        assert_eq!(
            slot.entry_bytes, row.entry_bytes,
            "level {level} entry width"
        );
        let want_leaf = if row.page_table {
            ps_code(PageSize(1 << row.va_lo))
        } else {
            KF_PS_NONE
        };
        assert_eq!(
            slot.leaf_ps, want_leaf,
            "level {level}: a leaf code exactly where the driver sets bPageTable (PD1 on GA10x, \
             PD0 always)"
        );
    }
}

/// The two leaf tables, the page-size table and the root alignment.
#[test]
fn the_shipped_ver2_leaf_tables_and_page_sizes_are_the_drivers() {
    let d = kf_format_ver2();
    let fmt = Ver2::ga10x();
    let (big, small) = (fmt.levels()[4], fmt.levels()[5]);
    assert_eq!(u32::from(d.big_va_lo), big.va_lo);
    assert_eq!(u32::from(d.small_va_lo), small.va_lo);
    assert_eq!(
        u32::from(d.big_entries),
        fmt.level_shift(4).unwrap().entries,
        "32, not 512"
    );
    assert_eq!(
        u32::from(d.small_entries),
        fmt.level_shift(5).unwrap().entries
    );
    assert_eq!(d.big_entry_bytes, big.entry_bytes);
    assert_eq!(d.small_entry_bytes, small.entry_bytes);
    assert_eq!(d.big_ps, ps_code(PageSize(1 << big.va_lo)));
    assert_eq!(d.small_ps, ps_code(PageSize(1 << small.va_lo)));

    let sizes = fmt.page_sizes();
    assert_eq!(sizes.len(), 4);
    let log2: Vec<u8> = sizes.iter().map(|s| s.0.trailing_zeros() as u8).collect();
    assert_eq!(
        d.ps_log2.to_vec(),
        log2,
        "ps_log2[code] is log2 of the size each code names"
    );
    for (code, s) in [(PS_4K, 0), (PS_64K, 1), (PS_2M, 2), (PS_512M, 3)] {
        assert_eq!(usize::from(code), s, "codes ascend with size");
    }
    assert_eq!(
        u64::from(d.root_align),
        1u64 << hdr::PDE_ADDRESS_SHIFT,
        "a directory base is a page address: the PDE address field's own granularity"
    );
}

/// Every bit position and field in the entry layout: valid, aperture, the four permission bits,
/// KIND and the four address fields, the last of which carry DIFFERENT shifts for the dual
/// entry's big half.
#[test]
fn the_shipped_ver2_entry_layout_is_the_header() {
    let d = kf_format_ver2();
    assert_eq!(u32::from(d.valid_bit), hdr::PTE_VALID.lo);
    assert_eq!(hdr::PTE_VALID.width(), 1);
    assert_eq!(u32::from(d.ap_lo), hdr::PTE_APERTURE.lo);
    assert_eq!(u32::from(d.ap_bits), hdr::PTE_APERTURE.width());
    assert_eq!(
        (hdr::PDE_APERTURE.lo, hdr::PDE_APERTURE.width()),
        (hdr::PTE_APERTURE.lo, hdr::PTE_APERTURE.width()),
        "the descriptor has ONE aperture field for PDE and PTE: the header must agree"
    );
    assert_eq!(u32::from(d.bit_volatile), hdr::PTE_VOL.lo);
    assert_eq!(u32::from(d.bit_privilege), hdr::PTE_PRIVILEGE.lo);
    assert_eq!(u32::from(d.bit_read_only), hdr::PTE_READ_ONLY.lo);
    assert_eq!(u32::from(d.bit_atomic_disable), hdr::PTE_ATOMIC_DISABLE.lo);
    assert_eq!(d.kind, field(hdr::PTE_KIND, 0));

    assert_eq!(
        d.addr_local,
        field(hdr::PTE_ADDRESS_VID, hdr::PTE_ADDRESS_SHIFT)
    );
    assert_eq!(
        d.addr_sys,
        field(hdr::PTE_ADDRESS_SYS, hdr::PTE_ADDRESS_SHIFT)
    );
    assert_eq!(
        d.big_addr_local,
        field(hdr::DUAL_ADDRESS_BIG_VID, hdr::DUAL_ADDRESS_BIG_SHIFT)
    );
    assert_eq!(
        d.big_addr_sys,
        field(hdr::DUAL_ADDRESS_BIG_SYS, hdr::DUAL_ADDRESS_BIG_SHIFT)
    );
    // The PDE and the dual entry's SMALL half (the HIGH qword, so bit 64 is its bit 0) use the
    // same two fields as a PTE, which is why the descriptor carries one pair.
    assert_eq!(
        field(hdr::PDE_ADDRESS_VID, hdr::PDE_ADDRESS_SHIFT),
        d.addr_local
    );
    assert_eq!(
        field(hdr::PDE_ADDRESS_SYS, hdr::PDE_ADDRESS_SHIFT),
        d.addr_sys
    );
    let small_vid = Bits {
        hi: hdr::DUAL_ADDRESS_SMALL_VID.hi - 64,
        lo: hdr::DUAL_ADDRESS_SMALL_VID.lo - 64,
    };
    let small_sys = Bits {
        hi: hdr::DUAL_ADDRESS_SMALL_SYS.hi - 64,
        lo: hdr::DUAL_ADDRESS_SMALL_SYS.lo - 64,
    };
    assert_eq!(field(small_vid, hdr::DUAL_ADDRESS_SHIFT), d.addr_local);
    assert_eq!(field(small_sys, hdr::DUAL_ADDRESS_SHIFT), d.addr_sys);
    assert_eq!(
        (
            hdr::DUAL_APERTURE_SMALL.lo - 64,
            hdr::DUAL_APERTURE_SMALL.width()
        ),
        (hdr::PTE_APERTURE.lo, hdr::PTE_APERTURE.width())
    );
}

/// The raw-nibble translation tables: a PTE's and a PDE's aperture are different tables, and the
/// descriptor's `addr_sel` picks the system-memory address field exactly for the sysmem apertures.
#[test]
fn the_shipped_ver2_aperture_tables_are_the_drivers() {
    let d = kf_format_ver2();
    let code = |a: Aperture| match a {
        Aperture::Vidmem => AP_VID,
        Aperture::Peer => AP_PEER,
        Aperture::SysmemCoherent => AP_SYS,
        Aperture::SysmemNonCoherent => AP_SYS_NC,
    };
    for raw in 0u64..4 {
        assert_eq!(
            d.pte_ap_map[raw as usize],
            code(pte_aperture(raw)),
            "PTE nibble {raw}"
        );
        assert_eq!(
            d.pde_ap_map[raw as usize],
            pde_aperture(raw).map_or(AP_INVALID, code),
            "PDE nibble {raw}"
        );
        let sys = pte_aperture(raw).code() >= 2;
        assert_eq!(
            d.addr_sel[raw as usize],
            u8::from(sys),
            "addr_sel nibble {raw}"
        );
    }
    assert_eq!(
        pde_aperture(u64::from(d.pde_ap_invalid)),
        None,
        "pde_ap_invalid names the PDE value that means 'no sub-level'"
    );
}

// =============================================================================================
// The independent builder, judged by the independent decoder
// =============================================================================================

/// Each encoder in `kf_cuda::synth` decodes to exactly what it was asked for.
#[test]
fn the_synth_encoders_decode_to_what_they_were_asked_for() {
    let f = Ver2::ga10x();
    let (pd2, pd0_big, small_pt) = (0x0004_5000u64, 0x0012_3400u64, 0x0006_7000u64);

    let Decode::Pde { edge, also } = f.decode_entry(1, u128::from(synth::pde(pd2))) else {
        panic!("synth::pde is a directory entry");
    };
    assert_eq!(
        (edge.next, edge.aperture, also),
        (pd2, Aperture::Vidmem, None)
    );

    // A dual entry built the way the fixture builds it: big half low, small half high.
    let dual = u128::from(synth::big_pde(pd0_big)) | (u128::from(synth::pde(small_pt)) << 64);
    let Decode::Pde { edge, also } = f.decode_entry(3, dual) else {
        panic!("a dual entry");
    };
    assert_eq!((edge.next, edge.child_level), (small_pt, 5));
    let also = also.expect("big half");
    assert_eq!(
        (also.next, also.child_level),
        (pd0_big, 4),
        "the big half's shift is 8"
    );

    // The absent half is genuinely absent, and a big half at offset 0 is a PRESENT table.
    let only_small = u128::from(synth::DUAL_HALF_ABSENT) | (u128::from(synth::pde(small_pt)) << 64);
    assert!(matches!(
        f.decode_entry(3, only_small),
        Decode::Pde { also: None, .. }
    ));
    assert!(matches!(
        f.decode_entry(3, u128::from(synth::big_pde(0))),
        Decode::Pde { .. }
    ));

    assert_eq!(
        f.decode_entry(5, u128::from(synth::pte(0x4000_0000))),
        Decode::Leaf {
            phys: 0x4000_0000,
            aperture: Aperture::Vidmem,
            size: PageSize(4096),
            read_only: false
        }
    );
    assert_eq!(
        f.decode_entry(5, u128::from(synth::pte_sys(0x1_2345_6000))),
        Decode::Leaf {
            phys: 0x1_2345_6000,
            aperture: Aperture::SysmemCoherent,
            size: PageSize(4096),
            read_only: false
        }
    );
    assert_eq!(
        f.decode_entry(5, u128::from(synth::sparse_pte())),
        Decode::Sparse
    );
}

/// The VA index helpers of `synth` are the driver's bit ranges.
#[test]
fn the_synth_va_index_helpers_are_the_drivers_bit_ranges() {
    let f = Ver2::ga10x();
    let va = 0x0001_80C0_E0A5_6000u64 & ((1 << 49) - 1);
    let idx = |level: usize| {
        let g = f.level_shift(level).unwrap();
        ((va >> g.shift) & u64::from(g.entries - 1)) as usize
    };
    assert_eq!(synth::vi3(va), idx(0));
    assert_eq!(synth::vi2(va), idx(1));
    assert_eq!(synth::vi1(va), idx(2));
    assert_eq!(synth::vi0(va), idx(3));
    assert_eq!(synth::vis(va), idx(5));
}

/// Walk the fixture image the isolate points the kernel at: the reference must find exactly the
/// `pages` contiguous 4 KiB mappings the builder promises, the one sparse slot after them, and
/// nothing else -- at origin 0 and at the 11.5 GiB where a real guest's roots sit.
#[test]
fn the_synth_fixture_walks_to_exactly_its_promised_mappings() {
    for origin in [0u64, 0x2_cea9_c000 & !0xFFF] {
        let (va, pages, phys) = (0x1_2000_0000u64, 8u64, 0x4000_0000u64);
        let (img, root, exp) = synth::contiguous_small_pages_at(origin, va, pages, phys);
        let mut r = Image {
            len: origin + img.mem.len() as u64,
            ..Image::default()
        };
        for (i, chunk) in img.mem.chunks(4096).enumerate() {
            if chunk.iter().any(|b| *b != 0) {
                let mut page = Box::new([0u8; 4096]);
                page[..chunk.len()].copy_from_slice(chunk);
                r.pages.insert(origin + i as u64 * 4096, page);
            }
        }
        let t = decode_subtree(
            &Ver2::ga10x(),
            &r,
            Page {
                phys: root,
                aperture: Aperture::Vidmem,
                level: 0,
                vabase: 0,
            },
            1 << 20,
            ZeroChild::IsATable,
        )
        .expect("within budget");
        assert!(t.faults.is_empty(), "origin {origin:#x}: {:?}", t.faults);
        assert_eq!(t.leaves.len() as u64, pages, "origin {origin:#x}");
        for (i, l) in t.leaves.iter().enumerate() {
            assert_eq!(l.va, exp.va + i as u64 * 4096);
            assert_eq!(l.phys, exp.gpga + i as u64 * 4096);
            assert_eq!(
                (l.size, l.aperture, l.read_only),
                (PageSize(4096), Aperture::Vidmem, false)
            );
        }
        assert_eq!(
            t.leaves.last().map(|l| l.va + l.size.0 - exp.va),
            Some(exp.len),
            "the run's length"
        );
        assert_eq!(
            t.sparse,
            vec![exp.va + pages * 4096],
            "the one declared-empty slot"
        );
    }
}
