//! ★★★ **`NV2080_CTRL_CMD_GPU_PROMOTE_CTX` — the decode and the address-table join**
//! (`#93`, `docs/design/gpu_promote_ctx.md`).
//!
//! # The oracle discipline, restated because this file is where it bites
//!
//! Three independent transcriptions of every message, per `gsp_core_bridge.md` §5.1:
//!
//! 1. `kayfabe_tests::rpcwire::promote_ctx_params` — a builder in a file that imports
//!    **nothing**, every offset a literal beside the `ogkm` line it came from;
//! 2. the offset-annotated **hand-written hex** in [`the_hand_written_blob_decodes`],
//!    which shares no code path with anything;
//! 3. `kayfabe_abi`'s decoder, whose offsets came from the same headers via a different
//!    reader, and which `crates/kayfabe-abi/tests/oracle_layout.rs` additionally pins
//!    against the **C artifact's** independently-written snoop offsets.
//!
//! Expected values are written out by hand. Nothing here asserts a count where the point
//! is *which*.
//!
//! # The seven C defects, and where each is subtracted
//!
//! | # | the C's behaviour | subtracted at |
//! |---|---|---|
//! | D1 | `entryCount` clamped to 64 (comment says 20; the header says 16) → 1536-byte over-read | [`the_entry_count_bound_is_16_and_it_is_refused_not_clamped`] |
//! | D2 | `bufferId` read 32 bits wide over a 16-bit field | `oracle_layout::the_c_artifacts_bufferid_bug_does_not_reproduce` + [`the_flag_bytes_cannot_reach_buffer_id`] |
//! | D3 | `!sz` silently swallows every promote-only entry | [`the_three_states_are_classified_by_content_not_dropped`] |
//! | D4 | aperture collapsed to a bool; undefined value 3 accepted as sysmem | [`the_aperture_is_total_and_three_is_refused_by_name`] |
//! | D5 | table keyed on `hChanClient` | [`two_procs_identical_vas_land_in_two_tables`] (keyed on the address space, by construction) |
//! | D6 | silent table-full drop at 1024 entries | not portable — `AddressTable` has no capacity; the *bound* that does exist is loud ([`more_ranges_than_the_core_bound_is_refused`]) |
//! | D7 | the reply clobbers the guest's params with a foreign-boot capture | [`a_case2_ack_writes_nothing_back`] |
//!
//! # What this file does NOT claim
//!
//! That promote-ctx is the road to first compute. It is not: the host owns and self-maps
//! the ranges these entries describe, and the compute working set's leaves arrive through
//! the observed CE page-table writes. This is a MISS = FAULT gap-filler for host-owned GR
//! context ranges — necessary, narrow, and nowhere near sufficient.

#![allow(clippy::unusual_byte_groupings)] // NVIDIA-shaped handle/VA literals

use std::sync::Arc;

use kayfabe_abi::GuestOs;
use kayfabe_abi::versions::{BENCH_DRIVER, ControlParams, DriverAbiTable, table_for};
use kayfabe_abi::view::{PromoteCensus, PromoteEntry};
use kayfabe_abi::wire::AbiError;
use kayfabe_arch::Aperture;
use kayfabe_arch::ids::{ControlCmd, GpuId, GpuVa, HClient, HObject, Pdb};
use kayfabe_core::gpa::GpaSpace;
use kayfabe_core::gpu::Gpu;
use kayfabe_core::promote::{
    CtxPromotion, MAX_PROMOTED_RANGES, PromoteDeclined, PromoteFault, PromotedRange,
};
use kayfabe_core::rmgraph::{AllocFacts, RmEvent};
use kayfabe_mmu::AddressFault;
use kayfabe_mocks::{MockIsolateFactory, WireClassArch, mock_classes, mock_ctrl};
use kayfabe_rmrpc::{BridgeRefusal, GraphPolicy, Translation};
use kayfabe_rt::device::{LockMode, SharedDevice};
use kayfabe_tests::rpcwire::{self as w, PromoteEntryWire, fn_id};
use kayfabe_tests::{Guarded, Scenario, identical_handles};

// =================================================================================
// Harness
// =================================================================================

fn abi() -> &'static DriverAbiTable {
    table_for(BENCH_DRIVER).expect("the bench driver is supported")
}

/// A complete (state-C) entry.
fn complete(va: u64, len: u64, phys: u64, phys_attr: u32, buffer_id: u16) -> PromoteEntryWire {
    PromoteEntryWire {
        gpu_phys_addr: phys,
        gpu_virt_addr: va,
        size: len,
        phys_attr,
        buffer_id,
        b_initialize: 1,
        b_nonmapped: 0,
    }
}

/// An initialize-only (state-A) entry: physical buffer, no VA, `bNonmapped` set.
fn initialize_only(len: u64, phys: u64, phys_attr: u32, buffer_id: u16) -> PromoteEntryWire {
    PromoteEntryWire {
        gpu_phys_addr: phys,
        gpu_virt_addr: 0,
        size: len,
        phys_attr,
        buffer_id,
        b_initialize: 1,
        b_nonmapped: 1,
    }
}

/// A promote-only (state-B) entry: VA only. `gpuPhysAddr`/`size` are the struct's
/// pre-zeroed values, i.e. fields this pass does not write.
fn promote_only(va: u64, buffer_id: u16) -> PromoteEntryWire {
    PromoteEntryWire {
        gpu_phys_addr: 0,
        gpu_virt_addr: va,
        size: 0,
        phys_attr: 0,
        buffer_id,
        b_initialize: 0,
        b_nonmapped: 0,
    }
}

fn decode(entries: &[PromoteEntryWire]) -> Result<Vec<PromoteEntry>, AbiError> {
    let count = u32::try_from(entries.len()).expect("small");
    let p = w::promote_ctx_params(1, 0xc1d0_0001, 0x5c00_0019, count, entries);
    abi().decode_promote_ctx(&p).map(|d| d.entries().collect())
}

// =================================================================================
// 1. The wire: three transcriptions
// =================================================================================

/// ★★ Transcription #2 — the same 560-byte params as a **hand-written, offset-annotated
/// byte array**, sharing no code path with the builder or the decoder.
///
/// The content is the repo's own captured blob, decoded in `gpu_promote_ctx.md` §2.4: a
/// real `PROMOTE_CTX` from a GA106 + 580 boot. Its 9 entries are 4 complete, 1
/// initialize-only and **4 promote-only** — the state the C artifact discards without a
/// name or a count.
#[test]
fn the_hand_written_blob_decodes() {
    let mut p = [0u8; 560];
    // ── header ───────────────────────────────────────────────────────────────────
    p[0..4].copy_from_slice(&[0x01, 0x00, 0x00, 0x00]); // +0   engineType  = 1 (GRAPHICS)
    p[4..8].copy_from_slice(&[0x00, 0x00, 0x00, 0x00]); // +4   hClient     = 0 (legacy)
    p[8..12].copy_from_slice(&[0x00, 0x00, 0x00, 0x00]); // +8   ChID
    p[12..16].copy_from_slice(&[0x09, 0x00, 0xe0, 0xc1]); // +12  hChanClient = 0xc1e00009
    p[16..20].copy_from_slice(&[0x45, 0x00, 0xba, 0xba]); // +16  hObject     = 0xbaba0045
    // +20 hVirtMemory, +24 virtAddress, +32 size — all zero (the legacy path is unused)
    p[40..44].copy_from_slice(&[0x09, 0x00, 0x00, 0x00]); // +40  entryCount  = 9

    // ── promoteEntry[0] @ +48 — bufId 0 MAIN, state C ────────────────────────────
    p[48..56].copy_from_slice(&[0x00, 0x60, 0xf9, 0xee, 0x02, 0, 0, 0]); // phys 0x2ef946000
    p[56..64].copy_from_slice(&[0x00, 0x00, 0x02, 0x20, 0x01, 0, 0, 0]); // va   0x120020000
    p[64..72].copy_from_slice(&[0x00, 0xa0, 0x0e, 0x00, 0, 0, 0, 0]); // size 0xea000
    p[72..76].copy_from_slice(&[0x04, 0x00, 0x00, 0x00]); // physAttr 0x4 (VIDMEM)
    p[76..78].copy_from_slice(&[0x00, 0x00]); // bufferId 0
    p[78] = 1; // bInitialize
    p[79] = 0; // bNonmapped
    // The corrected phys of entry 0 (the doc writes 0x2ef946000; encode it exactly).
    p[48..56].copy_from_slice(&0x2_ef94_6000u64.to_le_bytes());

    // ── promoteEntry[1] @ +80 — bufId 2 PATCH, state C ───────────────────────────
    p[80..88].copy_from_slice(&0x2_efa4_0000u64.to_le_bytes());
    p[88..96].copy_from_slice(&0x1_2010_a000u64.to_le_bytes());
    p[96..104].copy_from_slice(&0x4000u64.to_le_bytes());
    p[104..108].copy_from_slice(&4u32.to_le_bytes());
    p[108..110].copy_from_slice(&2u16.to_le_bytes());
    p[110] = 1;

    // ── promoteEntry[2..6] @ +112,144,176,208 — state B (VA only) ────────────────
    for (i, (va, buf)) in [
        (0x1_2019_0000u64, 3u16), // BUFFER_BUNDLE_CB
        (0x1_201a_0000, 4),       // PAGEPOOL
        (0x1_2100_0000, 5),       // ATTRIBUTE_CB
        (0x1_201c_0000, 6),       // RTV_CB_GLOBAL
    ]
    .into_iter()
    .enumerate()
    {
        let at = 48 + (2 + i) * 32;
        p[at + 8..at + 16].copy_from_slice(&va.to_le_bytes());
        p[at + 28..at + 30].copy_from_slice(&buf.to_le_bytes());
    }

    // ── promoteEntry[6] @ +240 — bufId 9 FECS_EVENT, state C, COH_SYS ────────────
    p[240..248].copy_from_slice(&0x1_0790_0000u64.to_le_bytes());
    p[248..256].copy_from_slice(&0x1_2001_0000u64.to_le_bytes());
    p[256..264].copy_from_slice(&0x10000u64.to_le_bytes());
    p[264..268].copy_from_slice(&5u32.to_le_bytes()); // 0x5 = COH_SYS | GPU_CACHEABLE_NO
    p[268..270].copy_from_slice(&9u16.to_le_bytes());
    p[270] = 1;

    // ── promoteEntry[7] @ +272 — bufId 10 PRIV_ACCESS_MAP, state A ───────────────
    p[272..280].copy_from_slice(&0x2_ef82_0000u64.to_le_bytes());
    p[288..296].copy_from_slice(&0x80000u64.to_le_bytes());
    p[296..300].copy_from_slice(&4u32.to_le_bytes());
    p[300..302].copy_from_slice(&10u16.to_le_bytes());
    p[302] = 1; // bInitialize
    p[303] = 1; // bNonmapped ★ — declares no VA, and says so

    // ── promoteEntry[8] @ +304 — bufId 11 UNRESTRICTED_PRIV_ACCESS_MAP, state C ──
    p[304..312].copy_from_slice(&0x2_eed8_0000u64.to_le_bytes());
    p[312..320].copy_from_slice(&0x1_2011_0000u64.to_le_bytes());
    p[320..328].copy_from_slice(&0x80000u64.to_le_bytes());
    p[328..332].copy_from_slice(&4u32.to_le_bytes());
    p[332..334].copy_from_slice(&11u16.to_le_bytes());
    p[334] = 1;

    let d = abi().decode_promote_ctx(&p).expect("decodes");
    assert_eq!(d.engine_type, 1);
    assert_eq!(d.h_chan_client, 0xc1e0_0009);
    assert_eq!(d.h_object, 0xbaba_0045);

    // ★ By exact CONTENT, entry by entry — "9 entries decoded" would be worthless here,
    // because the whole point is *which* nine.
    let got: Vec<PromoteEntry> = d.entries().collect();
    assert_eq!(
        got,
        vec![
            PromoteEntry::Promotable {
                va: 0x1_2002_0000,
                len: 0xea000,
                phys: 0x2_ef94_6000,
                aperture: Aperture::Vidmem,
                buffer_id: 0,
            },
            PromoteEntry::Promotable {
                va: 0x1_2010_a000,
                len: 0x4000,
                phys: 0x2_efa4_0000,
                aperture: Aperture::Vidmem,
                buffer_id: 2,
            },
            PromoteEntry::PromoteOnly {
                va: 0x1_2019_0000,
                buffer_id: 3
            },
            PromoteEntry::PromoteOnly {
                va: 0x1_201a_0000,
                buffer_id: 4
            },
            PromoteEntry::PromoteOnly {
                va: 0x1_2100_0000,
                buffer_id: 5
            },
            PromoteEntry::PromoteOnly {
                va: 0x1_201c_0000,
                buffer_id: 6
            },
            PromoteEntry::Promotable {
                va: 0x1_2001_0000,
                len: 0x10000,
                phys: 0x1_0790_0000,
                aperture: Aperture::SysmemCoherent,
                buffer_id: 9,
            },
            PromoteEntry::InitializeOnly {
                phys: 0x2_ef82_0000,
                len: 0x80000,
                aperture: Aperture::Vidmem,
                buffer_id: 10,
            },
            PromoteEntry::Promotable {
                va: 0x1_2011_0000,
                len: 0x80000,
                phys: 0x2_eed8_0000,
                aperture: Aperture::Vidmem,
                buffer_id: 11,
            },
        ]
    );
    assert_eq!(
        d.census(),
        PromoteCensus {
            promotable: 4,
            initialize_only: 1,
            promote_only: 4,
        },
        "★ five of nine entries are structurally unbindable — predicted from ogkm, \
         confirmed by this real capture",
    );

    // ★ Transcription #1 must agree with transcription #2, byte for byte.
    let built = w::promote_ctx_params(
        1,
        0xc1e0_0009,
        0xbaba_0045,
        9,
        &[
            complete(0x1_2002_0000, 0xea000, 0x2_ef94_6000, 4, 0),
            complete(0x1_2010_a000, 0x4000, 0x2_efa4_0000, 4, 2),
            promote_only(0x1_2019_0000, 3),
            promote_only(0x1_201a_0000, 4),
            promote_only(0x1_2100_0000, 5),
            promote_only(0x1_201c_0000, 6),
            complete(0x1_2001_0000, 0x10000, 0x1_0790_0000, 5, 9),
            initialize_only(0x80000, 0x2_ef82_0000, 4, 10),
            complete(0x1_2011_0000, 0x80000, 0x2_eed8_0000, 4, 11),
        ],
    );
    assert_eq!(
        built.as_slice(),
        p.as_slice(),
        "★ the independent builder and the hand-written blob are the same bytes",
    );
}

/// ★★★ **C defect D1.** The bound is 16, it comes from the header, and it is a
/// **refusal** — never a clamp.
///
/// The C clamped a guest-declared `entryCount` to 64 with a comment claiming the constant
/// is 20, and then read `params+560 … params+2096`: 1536 bytes past the struct, out of
/// the guest-writable 4096-byte queue element, straight into the table its hot resolution
/// path consults first.
///
/// **Swept, not witnessed**: every count in `0..=17`.
#[test]
fn the_entry_count_bound_is_16_and_it_is_refused_not_clamped() {
    for n in 0u32..=17 {
        // A full 16 entries are present in the buffer regardless, so a decoder that
        // clamped would find plausible content rather than obvious garbage — which is
        // exactly the shape that let the C's over-read go unnoticed.
        let entries: Vec<PromoteEntryWire> = (0..16)
            .map(|i| {
                complete(
                    0x1_2002_0000 + i * 0x1000,
                    0x1000,
                    0x2_0000_0000 + i * 0x1000,
                    0,
                    0,
                )
            })
            .collect();
        let p = w::promote_ctx_params(1, 0xc1d0_0001, 0x5c00_0019, n, &entries);
        let got = abi().decode_promote_ctx(&p);
        if n <= 16 {
            let d = got.expect("a declared count within the array decodes");
            assert_eq!(
                d.len(),
                n as usize,
                "★ exactly the declared count, never the buffer's content",
            );
        } else {
            assert_eq!(
                got.unwrap_err(),
                AbiError::PromoteEntryCount {
                    declared: n,
                    max: 16
                },
                "★ D1: refused by name at {n}, never clamped",
            );
        }
    }
    // …and the far end of the C's own clamp, plus a count that would run off any buffer.
    for n in [17u32, 20, 64, 65, 1000, u32::MAX] {
        let p = w::promote_ctx_params(1, 0xc1d0_0001, 0x5c00_0019, n, &[]);
        assert_eq!(
            abi().decode_promote_ctx(&p),
            Err(AbiError::PromoteEntryCount {
                declared: n,
                max: 16
            }),
            "count {n}",
        );
    }
}

/// The length sweep every decoder gets: **refuse short at every length below `SIZE`**,
/// accept long.
#[test]
fn short_is_refused_at_every_length_and_long_is_accepted() {
    let p = w::promote_ctx_params(
        1,
        0xc1d0_0001,
        0x5c00_0019,
        1,
        &[complete(0x1_2002_0000, 0x1000, 0x2_0000_0000, 0, 7)],
    );
    assert_eq!(p.len(), 560);
    for n in 0..560 {
        assert_eq!(
            abi().decode_promote_ctx(&p[..n]),
            Err(AbiError::Truncated {
                c_name: "NV2080_CTRL_GPU_PROMOTE_CTX_PARAMS",
                need: 560,
                got: n,
            }),
            "★ a {n}-byte buffer must not zero-extend into a plausible struct",
        );
    }
    abi()
        .decode_promote_ctx(&p)
        .expect("exactly 560 bytes decodes");
    let mut long = p.clone();
    long.extend_from_slice(&[0xAB; 64]);
    assert_eq!(
        abi()
            .decode_promote_ctx(&long)
            .expect("a longer buffer is a legitimate newer-ABI image")
            .entries()
            .collect::<Vec<_>>(),
        abi()
            .decode_promote_ctx(&p)
            .expect("decodes")
            .entries()
            .collect::<Vec<_>>(),
    );
}

/// ★★ **C defect D3 subtracted.** The three states are classified by content; none is
/// dropped, and the two unbindable ones are *counted*.
///
/// **The cross-product, not one fixture per state**: each of the three states over two
/// different buffer ids, in every position.
#[test]
fn the_three_states_are_classified_by_content_not_dropped() {
    /// A state's name and the builder that produces it.
    type State = (&'static str, fn(u64, u16) -> PromoteEntryWire);
    let states: [State; 3] = [
        ("C", |va, id| {
            complete(va, 0x2000, 0x3_0000_0000 + va, 1, id)
        }),
        ("A", |va, id| {
            initialize_only(0x2000, 0x3_0000_0000 + va, 2, id)
        }),
        ("B", promote_only),
    ];
    for (i, (na, fa)) in states.iter().enumerate() {
        for (j, (nb, fb)) in states.iter().enumerate() {
            for &(id_a, id_b) in &[(0u16, 2u16), (10, 11)] {
                let va_a = 0x1_2002_0000 + (i as u64) * 0x10000;
                let va_b = 0x1_2102_0000 + (j as u64) * 0x10000;
                let got = decode(&[fa(va_a, id_a), fb(va_b, id_b)]).expect("decodes");
                assert_eq!(got.len(), 2, "{na}+{nb}: both entries survive decoding");
                let want = |n: &str, va: u64, id: u16| match n {
                    "C" => PromoteEntry::Promotable {
                        va,
                        len: 0x2000,
                        phys: 0x3_0000_0000 + va,
                        aperture: Aperture::SysmemCoherent,
                        buffer_id: id,
                    },
                    "A" => PromoteEntry::InitializeOnly {
                        phys: 0x3_0000_0000 + va,
                        len: 0x2000,
                        aperture: Aperture::SysmemNonCoherent,
                        buffer_id: id,
                    },
                    _ => PromoteEntry::PromoteOnly { va, buffer_id: id },
                };
                assert_eq!(got[0], want(na, va_a, id_a), "{na}+{nb} first");
                assert_eq!(got[1], want(nb, va_b, id_b), "{na}+{nb} second");
                assert_eq!(
                    got[0].buffer_id(),
                    id_a,
                    "the buffer id survives every state",
                );
                assert_eq!(got[1].buffer_id(), id_b);
            }
        }
    }
}

/// ★★ **`bNonmapped` dominates the VALUE.** A hostile guest can set the flag *and* a
/// plausible VA and length; a value-first classifier would bind it.
#[test]
fn b_nonmapped_is_never_promotable_however_plausible_the_values() {
    let hostile = PromoteEntryWire {
        gpu_phys_addr: 0x2_ef94_6000,
        gpu_virt_addr: 0x1_2002_0000, // a real GR context VA
        size: 0xea000,                // a real length
        phys_attr: 0,
        buffer_id: 0,
        b_initialize: 1,
        b_nonmapped: 1, // ★ …and the flag that says "do not promote this VA"
    };
    assert_eq!(
        decode(&[hostile]).expect("decodes"),
        vec![PromoteEntry::InitializeOnly {
            phys: 0x2_ef94_6000,
            len: 0xea000,
            aperture: Aperture::Vidmem,
            buffer_id: 0,
        }],
    );
}

/// ★★ **C defect D2, at the classifier.** `bInitialize`/`bNonmapped` live at +30/+31,
/// inside the 32-bit word the C read at +28. The two flag bytes must be unable to reach
/// `buffer_id`.
#[test]
fn the_flag_bytes_cannot_reach_buffer_id() {
    // Same `bufferId`, every combination of the two flag bytes. The C's four-byte read
    // would have produced 0x0000_00A5, 0x0001_00A5, 0x0100_00A5 and 0x0101_00A5.
    for (init, nonmapped) in [(0u8, 0u8), (1, 0), (0, 1), (1, 1), (0xFF, 0xFF)] {
        let e = PromoteEntryWire {
            b_initialize: init,
            b_nonmapped: nonmapped,
            ..complete(0x1_2002_0000, 0x1000, 0x2_0000_0000, 0, 0x00A5)
        };
        assert_eq!(
            decode(&[e]).expect("decodes")[0].buffer_id(),
            0x00A5,
            "★ D2: bInitialize={init} bNonmapped={nonmapped} must not spill into bufferId",
        );
    }
    // …while the flags themselves still change the meaning of the entry, so this is not
    // a test that passes by the fields being ignored.
    let mapped = complete(0x1_2002_0000, 0x1000, 0x2_0000_0000, 0, 0x00A5);
    let unmapped = PromoteEntryWire {
        b_nonmapped: 1,
        ..mapped
    };
    assert_ne!(
        decode(&[mapped]).expect("decodes")[0],
        decode(&[unmapped]).expect("decodes")[0],
    );
    // And the top byte of the u16 is a real bufferId, not a flag spill.
    assert_eq!(
        decode(&[promote_only(0x1_2002_0000, 0xFF00)]).expect("decodes")[0].buffer_id(),
        0xFF00,
    );
}

/// ★★ **C defect D4 subtracted.** `physAttr[1:0]` decodes totally into
/// `kayfabe_arch::Aperture`, and the undefined value `3` is a **named refusal** rather
/// than being folded into sysmem.
///
/// ★ And the refusal fires only where the state carries an aperture. The promote preparer
/// never writes `physAttr`, so on a promote-only entry the field is the struct's
/// pre-zeroed value — refusing on it would be reading an absence as a fact, which is the
/// mistake this whole decoder exists to avoid one field over.
#[test]
fn the_aperture_is_total_and_three_is_refused_by_name() {
    for (bits, want) in [
        (0u32, Aperture::Vidmem),
        (1, Aperture::SysmemCoherent),
        (2, Aperture::SysmemNonCoherent),
    ] {
        // The upper bits are other fields (GPU_CACHEABLE, PRESERVE_CTX) and must not
        // change the aperture.
        for extra in [0u32, 0x4, 0x8, 0xFFFF_FFFC] {
            let attr = bits | extra;
            let got = decode(&[complete(0x1_2002_0000, 0x1000, 0x2_0000_0000, attr, 1)])
                .expect("decodes");
            assert_eq!(
                got[0],
                PromoteEntry::Promotable {
                    va: 0x1_2002_0000,
                    len: 0x1000,
                    phys: 0x2_0000_0000,
                    aperture: want,
                    buffer_id: 1,
                },
                "physAttr {attr:#x}",
            );
        }
    }
    // 3 — undefined. Refused on both states that carry an aperture, at the entry index.
    for (idx, e) in [
        (0usize, complete(0x1_2002_0000, 0x1000, 0x2_0000_0000, 3, 1)),
        (0, initialize_only(0x1000, 0x2_0000_0000, 3, 1)),
    ] {
        assert_eq!(
            decode(&[e]),
            Err(AbiError::PromoteAperture {
                entry: idx,
                phys_attr: 3
            }),
        );
    }
    // The index is the entry's own, not a constant.
    let ok = complete(0x1_2002_0000, 0x1000, 0x2_0000_0000, 0, 1);
    let bad = complete(0x1_2003_0000, 0x1000, 0x2_0001_0000, 0x7, 2);
    assert_eq!(
        decode(&[ok, ok, bad]),
        Err(AbiError::PromoteAperture {
            entry: 2,
            phys_attr: 0x7
        }),
    );
    // …and a promote-only entry carries no aperture, so a garbage `physAttr` there is
    // not a fact and not a refusal.
    let attrless = PromoteEntryWire {
        phys_attr: 3,
        ..promote_only(0x1_2002_0000, 5)
    };
    assert_eq!(
        decode(&[attrless]).expect("a field the producer never wrote is not a fact"),
        vec![PromoteEntry::PromoteOnly {
            va: 0x1_2002_0000,
            buffer_id: 5
        }],
    );
}

/// The **legacy** `hVirtMemory`/`(virtAddress, size)` shape is refused rather than
/// guessed at. Neither real producer emits it.
#[test]
fn the_legacy_shape_is_refused_by_name() {
    for (hvm, va, sz) in [
        (0x7a50_0001u32, 0u64, 0u64),
        (0, 0x1_2002_0000, 0),
        (0, 0, 0x1000),
        (0x7a50_0001, 0x1_2002_0000, 0x1000),
    ] {
        let p = w::promote_ctx_legacy_params(0xc1d0_0001, 0x5c00_0019, hvm, va, sz);
        assert_eq!(
            abi().decode_promote_ctx(&p),
            Err(AbiError::PromoteLegacyShape {
                h_virt_memory: hvm,
                virt_address: va,
                size: sz,
            }),
        );
    }
    // A well-formed entry-count-zero control with the legacy fields also zero is legal
    // and decodes to nothing — that is not the same message.
    let empty = w::promote_ctx_params(1, 0xc1d0_0001, 0x5c00_0019, 0, &[]);
    let d = abi().decode_promote_ctx(&empty).expect("decodes");
    assert!(d.is_empty());
    assert_eq!(d.census(), PromoteCensus::default());
}

/// `paramsSize` is checked **exactly**, not as a lower bound, and the number is composed
/// rather than typed.
#[test]
fn the_declared_params_size_is_exact() {
    assert_eq!(
        abi().control_params(ControlCmd(0x2080_012b)),
        Some(ControlParams::PromoteCtx),
    );
    assert_eq!(ControlParams::PromoteCtx.params_size(), Some(560));
}

// =================================================================================
// 2. The join, against a bare `Gpu`
// =================================================================================

const A_CLIENT: HClient = HClient(0xAA);
const B_CLIENT: HClient = HClient(0xBB);
const A_PDB: Pdb = Pdb(0x3401_000);
const B_PDB: Pdb = Pdb(0x3405_000);
/// The GR context-buffer VA both processes use — the #14 shape, and the real
/// deterministic GR base the guest driver picks.
const GR_VA: GpuVa = GpuVa(0x1_2002_0000);
const GR_LEN: u64 = 0xea000;

fn world() -> Guarded<Gpu> {
    let (factory, rec) = MockIsolateFactory::new();
    let gpa = GpaSpace::new(0x1_0000_0000..0x100_0000_0000, 0x1_0000_0000);
    let mut gpu =
        Gpu::new(Box::new(WireClassArch::new()), Box::new(factory), gpa).expect("device realizes");
    let mut s = Scenario::new();
    s.compute_process(A_CLIENT, A_PDB, identical_handles(0x10, 0x11));
    s.compute_process(B_CLIENT, B_PDB, identical_handles(0x20, 0x21));
    for ev in s.events {
        gpu.apply(ev).expect("scenario applies cleanly");
    }
    Guarded::new("promote_ctx::world", gpu, rec)
}

/// The handles `identical_handles` mints — both procs use the SAME values, which is the
/// point.
const H_GR_CHANNEL: HObject = HObject(0x5c00_0019);
const H_TSG: HObject = HObject(0x5c00_0012);
const H_DEVICE: HObject = HObject(0x5c00_0001);

fn promotion(client: HClient, object: HObject, ranges: Vec<PromotedRange>) -> CtxPromotion {
    CtxPromotion {
        client,
        chan_client: client,
        object,
        ranges,
        declined: PromoteDeclined::default(),
    }
}

fn gr_range(va: GpuVa, phys: u64) -> PromotedRange {
    PromotedRange {
        va,
        len: GR_LEN,
        phys,
        aperture: Aperture::Vidmem,
        buffer_id: 0,
    }
}

fn pid_of(gpu: &Gpu, pdb: Pdb) -> kayfabe_core::ProcId {
    *gpu.spine
        .by_pdb
        .get(&(GpuId::ZERO, pdb))
        .expect("routed by its PDB")
}

fn resolve_in(gpu: &Gpu, pdb: Pdb, va: GpuVa) -> Result<u64, AddressFault> {
    let pid = pid_of(gpu, pdb);
    gpu.procs[&pid].vases[&(GpuId::ZERO, pdb)]
        .table
        .resolve(pdb, va)
        .map(|(b, off)| b.phys + off)
}

/// The base case, stated as MISS-then-HIT so the binding is what changed.
#[test]
fn a_promotion_binds_into_the_address_space_its_object_names() {
    let mut gpu = world();
    assert_eq!(
        resolve_in(&gpu, A_PDB, GR_VA),
        Err(AddressFault::Miss {
            pdb: A_PDB,
            va: GR_VA
        }),
        "★ MISS = FAULT before the promotion — this is the gap the join fills",
    );

    let join = gpu
        .promote_ctx(&promotion(
            A_CLIENT,
            H_GR_CHANNEL,
            vec![gr_range(GR_VA, 0x2_ef94_6000)],
        ))
        .expect("the promotion joins");
    assert_eq!(join.bound, 1);
    assert_eq!(join.already, 0);
    assert_eq!(join.route.pdb, A_PDB);
    assert_eq!(join.route.proc, pid_of(&gpu, A_PDB));

    assert_eq!(resolve_in(&gpu, A_PDB, GR_VA), Ok(0x2_ef94_6000));
    // …and the offset arithmetic inside the binding is real.
    assert_eq!(
        resolve_in(&gpu, A_PDB, GpuVa(GR_VA.0 + 0x1000)),
        Ok(0x2_ef94_7000),
    );
    // The binding is declared-only: nothing host-side exists and nothing needs reclaiming.
    let pid = pid_of(&gpu, A_PDB);
    let (b, _) = gpu.procs[&pid].vases[&(GpuId::ZERO, A_PDB)]
        .table
        .resolve(A_PDB, GR_VA)
        .expect("bound");
    assert_eq!(b.host, None);
    assert_eq!(b.aperture, Aperture::Vidmem);
    // …and it is in the PROMOTE idempotence set, not the RPC one.
    let vas = &gpu.procs[&pid].vases[&(GpuId::ZERO, A_PDB)];
    assert!(vas.promote_bound.contains(&GR_VA.0));
    assert!(!vas.rpc_bound.contains(&GR_VA.0));
}

/// `hObject` may be a channel **or** a channel group, and both resolve to the same
/// address space.
#[test]
fn a_tsg_named_as_the_context_object_resolves_to_the_same_vas() {
    let mut gpu = world();
    let via_tsg = gpu
        .promote_ctx(&promotion(
            A_CLIENT,
            H_TSG,
            vec![gr_range(GR_VA, 0x2_ef94_6000)],
        ))
        .expect("a TSG is a legal context object");
    assert_eq!(via_tsg.route.pdb, A_PDB);

    let mut gpu2 = world();
    let via_chan = gpu2
        .promote_ctx(&promotion(
            A_CLIENT,
            H_GR_CHANNEL,
            vec![gr_range(GR_VA, 0x2_ef94_6000)],
        ))
        .expect("a channel is a legal context object");
    assert_eq!(
        via_tsg.route, via_chan.route,
        "★ the group and its channel name ONE address space",
    );
}

/// Every routing refusal, by exact variant.
#[test]
fn the_routing_refusals_are_named() {
    let mut gpu = world();
    // A handle nobody allocated.
    assert_eq!(
        gpu.promote_ctx(&promotion(A_CLIENT, HObject(0xDEAD), vec![])),
        Err(PromoteFault::UnknownContextObject {
            client: A_CLIENT,
            object: HObject(0xDEAD),
        }),
    );
    // A handle in a client that never declared itself.
    assert_eq!(
        gpu.promote_ctx(&CtxPromotion {
            chan_client: HClient(0xCC),
            ..promotion(A_CLIENT, H_GR_CHANNEL, vec![])
        }),
        Err(PromoteFault::UnknownContextObject {
            client: HClient(0xCC),
            object: H_GR_CHANNEL,
        }),
    );
    // A real handle of the wrong KIND — typed resolution (decision #18C). A hostile
    // guest naming its Device must not have the promotion land wherever that Device's
    // neighbour happens to live.
    assert_eq!(
        gpu.promote_ctx(&promotion(A_CLIENT, H_DEVICE, vec![])),
        Err(PromoteFault::NotAContextObject {
            client: A_CLIENT,
            object: H_DEVICE,
        }),
    );
}

/// ★ A promotion naming a VAS that has since **died**. The channel handle is still there,
/// the address space is not, and the answer is a named refusal rather than a plausible
/// landing site.
#[test]
fn a_promotion_naming_a_dead_vas_is_refused() {
    let mut gpu = world();
    // Sanity: it routes while the VASpace lives.
    gpu.promote_ctx(&promotion(
        A_CLIENT,
        H_GR_CHANNEL,
        vec![gr_range(GR_VA, 0x2_ef94_6000)],
    ))
    .expect("routes before the free");

    gpu.apply(RmEvent::Free {
        client: A_CLIENT,
        handle: HObject(0x5c00_0010), // the VASpace
    })
    .expect("the guest may free its VASpace");

    assert_eq!(
        gpu.promote_ctx(&promotion(
            A_CLIENT,
            H_GR_CHANNEL,
            vec![gr_range(GpuVa(0x1_2100_0000), 0x2_0000_0000)],
        )),
        Err(PromoteFault::ContextVasUndeclared {
            client: A_CLIENT,
            object: H_GR_CHANNEL,
        }),
        "★ the channel is still live; its address space is not",
    );
}

/// ★★★ **The cross-namespace guard the C could not even express.**
///
/// `hChanClient`/`hObject` name someone's channel; the envelope's `hClient` says who is
/// asking. Without checking one against the other, a client declares bindings in a
/// victim's address space by naming the victim's handles — and the C read only
/// `hChanClient`, so it had nothing to compare.
#[test]
fn a_foreign_acting_client_cannot_promote_into_another_procs_address_space() {
    let mut gpu = world();
    let owner = pid_of(&gpu, A_PDB);
    // B names A's client/channel for resolution, but acts as itself.
    assert_eq!(
        gpu.promote_ctx(&CtxPromotion {
            client: B_CLIENT,      // envelope: the attacker
            chan_client: A_CLIENT, // params: the victim's namespace
            object: H_GR_CHANNEL,
            ranges: vec![gr_range(GR_VA, 0xBADD_0000)],
            declined: PromoteDeclined::default(),
        }),
        Err(PromoteFault::ForeignContextObject {
            client: B_CLIENT,
            object: H_GR_CHANNEL,
            owner,
        }),
    );
    assert_eq!(
        resolve_in(&gpu, A_PDB, GR_VA),
        Err(AddressFault::Miss {
            pdb: A_PDB,
            va: GR_VA
        }),
        "★ and nothing was written on the way to the refusal",
    );
    // The rule is component-scoped, not handle-scoped: the two clients being DIFFERENT
    // is not itself the offence — the envelope client being outside the owning component
    // is. A's own promotion with the same two handles is accepted.
    gpu.promote_ctx(&promotion(
        A_CLIENT,
        H_GR_CHANNEL,
        vec![gr_range(GR_VA, 0x2_ef94_6000)],
    ))
    .expect("the owner may promote into its own address space");
}

/// The all-or-nothing property, and every content refusal by exact variant.
#[test]
fn a_promotion_that_would_half_apply_is_refused_whole() {
    // Two ranges inside ONE promotion that overlap each other.
    let mut gpu = world();
    assert_eq!(
        gpu.promote_ctx(&promotion(
            A_CLIENT,
            H_GR_CHANNEL,
            vec![
                gr_range(GR_VA, 0x2_ef94_6000),
                gr_range(GpuVa(GR_VA.0 + 0x1000), 0x2_0000_0000),
            ]
        )),
        Err(PromoteFault::SelfOverlap {
            a: GR_VA,
            b: GpuVa(GR_VA.0 + 0x1000),
        }),
    );
    assert_eq!(
        resolve_in(&gpu, A_PDB, GR_VA),
        Err(AddressFault::Miss {
            pdb: A_PDB,
            va: GR_VA
        }),
        "★ the FIRST range was valid on its own and is still not bound",
    );

    // A range that PARTIALLY overlaps a live binding.
    gpu.promote_ctx(&promotion(
        A_CLIENT,
        H_GR_CHANNEL,
        vec![gr_range(GR_VA, 0x2_ef94_6000)],
    ))
    .expect("the first promotion binds");
    let straddle = PromotedRange {
        va: GpuVa(GR_VA.0 + 0x1000),
        len: 0x2000,
        phys: 0x4_0000_0000,
        aperture: Aperture::Vidmem,
        buffer_id: 2,
    };
    assert_eq!(
        gpu.promote_ctx(&promotion(A_CLIENT, H_GR_CHANNEL, vec![straddle])),
        Err(PromoteFault::Collides {
            va: straddle.va,
            len: straddle.len,
        }),
    );
    // A range at the SAME start with DIFFERENT contents — unmap is eager, so silently
    // replacing would hide the collision class.
    assert_eq!(
        gpu.promote_ctx(&promotion(
            A_CLIENT,
            H_GR_CHANNEL,
            vec![gr_range(GR_VA, 0xFFFF_0000)]
        )),
        Err(PromoteFault::Collides {
            va: GR_VA,
            len: GR_LEN
        }),
    );
    assert_eq!(
        resolve_in(&gpu, A_PDB, GR_VA),
        Ok(0x2_ef94_6000),
        "★ the original binding is untouched by either refusal",
    );

    // Malformed ranges: zero length and a wrapping end. Guest-influenced arithmetic
    // refuses; it never panics and never clips.
    for r in [
        PromotedRange {
            len: 0,
            ..gr_range(GpuVa(0x1_3000_0000), 1)
        },
        PromotedRange {
            va: GpuVa(u64::MAX - 0x100),
            len: 0x1000,
            ..gr_range(GpuVa(0), 1)
        },
    ] {
        assert_eq!(
            gpu.promote_ctx(&promotion(A_CLIENT, H_GR_CHANNEL, vec![r])),
            Err(PromoteFault::Malformed {
                va: r.va,
                len: r.len
            }),
        );
    }
}

/// An identical **re-promote** is idempotent, not a collision. The same context buffer is
/// promoted again when a second channel of a TSG comes up.
#[test]
fn an_identical_repromote_is_idempotent() {
    let mut gpu = world();
    let p = promotion(A_CLIENT, H_GR_CHANNEL, vec![gr_range(GR_VA, 0x2_ef94_6000)]);
    let first = gpu.promote_ctx(&p).expect("binds");
    assert_eq!((first.bound, first.already), (1, 0));
    for _ in 0..3 {
        let again = gpu.promote_ctx(&p).expect("re-promotes");
        assert_eq!(
            (again.bound, again.already),
            (0, 1),
            "★ counted as already-bound, and nothing is rebound",
        );
    }
    // Through the TSG handle too — the same VAS, the same ranges.
    let via_tsg = gpu
        .promote_ctx(&promotion(
            A_CLIENT,
            H_TSG,
            vec![gr_range(GR_VA, 0x2_ef94_6000)],
        ))
        .expect("the group's promotion is the same fact");
    assert_eq!((via_tsg.bound, via_tsg.already), (0, 1));
    assert_eq!(resolve_in(&gpu, A_PDB, GR_VA), Ok(0x2_ef94_6000));
}

/// A control carrying **only** unbindable entries produces no binding, no fault, and a
/// census that says what happened. Refusing it would reject legitimate guest traffic;
/// dropping it silently is C defect D3.
#[test]
fn unbindable_entries_produce_no_binding_and_no_fault() {
    let mut gpu = world();
    let join = gpu
        .promote_ctx(&CtxPromotion {
            client: A_CLIENT,
            chan_client: A_CLIENT,
            object: H_GR_CHANNEL,
            ranges: vec![],
            declined: PromoteDeclined {
                initialize_only: 1,
                promote_only: 4,
            },
        })
        .expect("★ a legal guest sequence must not return a fault");
    assert_eq!(join.bound, 0);
    assert_eq!(join.already, 0);
    assert_eq!(
        join.declined,
        PromoteDeclined {
            initialize_only: 1,
            promote_only: 4,
        },
        "★ named and counted, never silent",
    );
    assert_eq!(
        resolve_in(&gpu, A_PDB, GpuVa(0x1_2019_0000)),
        Err(AddressFault::Miss {
            pdb: A_PDB,
            va: GpuVa(0x1_2019_0000),
        }),
        "★ a promote-only VA is NOT bound to physical zero — that would be manufacturing \
         an address",
    );
}

/// The core's own bound is loud. It is deliberately a **second** number from the ABI's:
/// the C artifact had three that disagreed silently and the largest one won.
#[test]
fn more_ranges_than_the_core_bound_is_refused() {
    let mut gpu = world();
    let ranges: Vec<PromotedRange> = (0..=MAX_PROMOTED_RANGES as u64)
        .map(|i| PromotedRange {
            va: GpuVa(0x1_2002_0000 + i * 0x10000),
            len: 0x1000,
            phys: 0x2_0000_0000 + i * 0x1000,
            aperture: Aperture::Vidmem,
            buffer_id: 0,
        })
        .collect();
    assert_eq!(
        gpu.promote_ctx(&promotion(A_CLIENT, H_GR_CHANNEL, ranges.clone())),
        Err(PromoteFault::TooManyRanges {
            declared: MAX_PROMOTED_RANGES + 1,
            max: MAX_PROMOTED_RANGES,
        }),
    );
    // Exactly the bound is fine.
    let join = gpu
        .promote_ctx(&promotion(
            A_CLIENT,
            H_GR_CHANNEL,
            ranges[..MAX_PROMOTED_RANGES].to_vec(),
        ))
        .expect("the bound itself is legal");
    assert_eq!(join.bound as usize, MAX_PROMOTED_RANGES);
}

/// ★★★ **#14, and C defect D5.** Two procs promote the *same* guest VA. The bindings land
/// in two distinct tables and neither resolves the other's physical address.
///
/// The C keyed its table on the `hChanClient` it read out of the params — and hardware
/// has since measured that two concurrent CUDA processes **share** one duplicated client,
/// so that key aliases them together. Here the key is the address space.
#[test]
fn two_procs_identical_vas_land_in_two_tables() {
    let mut gpu = world();
    gpu.promote_ctx(&promotion(
        A_CLIENT,
        H_GR_CHANNEL,
        vec![gr_range(GR_VA, 0xA000_0000)],
    ))
    .expect("A promotes");
    gpu.promote_ctx(&promotion(
        B_CLIENT,
        H_GR_CHANNEL, // ★ the SAME handle value — `identical_handles`
        vec![gr_range(GR_VA, 0xB000_0000)],
    ))
    .expect("B promotes the identical guest VA");

    assert_eq!(resolve_in(&gpu, A_PDB, GR_VA), Ok(0xA000_0000));
    assert_eq!(resolve_in(&gpu, B_PDB, GR_VA), Ok(0xB000_0000));
    assert_ne!(pid_of(&gpu, A_PDB), pid_of(&gpu, B_PDB));
}

/// ★★ **The `rpc_bound` reaping trap** (`gpu_promote_ctx.md` §6.1). A promote binding
/// filed under the RPC map source's idempotence set would be unbound on the very next
/// `Spine::apply` — a table correct immediately after the control and empty a moment
/// later, which reads as a race.
///
/// This test is the only thing that would catch it, and it fails loudly if someone reuses
/// the set.
#[test]
fn promote_bindings_survive_a_subsequent_spine_apply() {
    let mut gpu = world();
    gpu.promote_ctx(&promotion(
        A_CLIENT,
        H_GR_CHANNEL,
        vec![gr_range(GR_VA, 0x2_ef94_6000)],
    ))
    .expect("binds");
    assert_eq!(resolve_in(&gpu, A_PDB, GR_VA), Ok(0x2_ef94_6000));

    // Any subsequent event re-runs the whole derivation, including
    // `sync_rpc_mappings`'s stale-unbind pass over `rpc_bound`.
    for i in 0..4u32 {
        gpu.apply(RmEvent::Alloc {
            client: A_CLIENT,
            parent: H_DEVICE,
            handle: HObject(0x5c00_0100 + i),
            class: mock_classes::VASPACE,
            facts: AllocFacts::default(),
        })
        .expect("applies");
        assert_eq!(
            resolve_in(&gpu, A_PDB, GR_VA),
            Ok(0x2_ef94_6000),
            "★ §6.1: the promotion must survive apply #{i}",
        );
    }
}

/// ★ The ownership index is **derived, never accreted** — it survives a proc teardown by
/// being rebuilt, and the surviving proc still routes.
#[test]
fn the_ownership_index_survives_a_proc_teardown() {
    let mut gpu = world();
    gpu.promote_ctx(&promotion(
        A_CLIENT,
        H_GR_CHANNEL,
        vec![gr_range(GR_VA, 0xA000_0000)],
    ))
    .expect("A promotes");
    gpu.promote_ctx(&promotion(
        B_CLIENT,
        H_GR_CHANNEL,
        vec![gr_range(GR_VA, 0xB000_0000)],
    ))
    .expect("B promotes");

    let a_pid = pid_of(&gpu, A_PDB);
    assert!(gpu.retire_proc(a_pid), "A retires");
    let _ = gpu.reap_retired();

    // B's route is untouched, and B can still promote.
    assert_eq!(resolve_in(&gpu, B_PDB, GR_VA), Ok(0xB000_0000));
    gpu.promote_ctx(&promotion(
        B_CLIENT,
        H_GR_CHANNEL,
        vec![PromotedRange {
            va: GpuVa(0x1_2100_0000),
            len: 0x1000,
            phys: 0xB100_0000,
            aperture: Aperture::Vidmem,
            buffer_id: 5,
        }],
    ))
    .expect("B still promotes after A dies");

    // A's own address space is gone, and a promotion naming it is refused by name
    // rather than landing on whoever inherited the id.
    assert!(
        !gpu.spine.by_pdb.contains_key(&(GpuId::ZERO, A_PDB)),
        "A's PDB left the routing map with its proc",
    );
}

/// ★★ **The ownership index is DERIVED, and a proc teardown is the wrong place to look.**
///
/// Retiring a proc does not free its RM objects — the guest's graph still holds the
/// client and its channels — so `ctx_vas` keeping A's rows across
/// [`the_ownership_index_survives_a_proc_teardown`] is the index tracking the projection,
/// not accreting. The property only becomes observable when a context object leaves the
/// **graph**, and a stale row is unreachable through [`route_promote_ctx`] anyway (the
/// handle no longer resolves, so `node.id()` is never produced), which is exactly why
/// nothing saw it.
///
/// Found by `scripts/bite_promote_ctx.py` at rev 4a93d54: deleting `Spine::refresh`'s
/// `ctx_vas.clear()` was a NON-BITER against all 25 tests this target had. Unlike §9.5's
/// redundant filter, the line is not deletable — without it the map keeps one row per
/// context object ever seen, for the lifetime of the device — so the answer is the test
/// that pins it rather than the deletion.
#[test]
fn a_freed_context_object_leaves_the_ownership_index() {
    let mut gpu = world();
    let rows = |g: &Gpu, pdb: Pdb| g.spine.ctx_vas.values().filter(|&&(_, p)| p == pdb).count();
    let before = rows(&gpu, A_PDB);
    assert!(before >= 1, "A's context objects are indexed to begin with");
    assert_eq!(rows(&gpu, B_PDB), before, "…and so are B's, identically");

    gpu.apply(RmEvent::Free {
        client: A_CLIENT,
        handle: H_GR_CHANNEL,
    })
    .expect("the guest frees its channel");

    assert_eq!(
        rows(&gpu, A_PDB),
        before - 1,
        "★ the freed channel's row is GONE from the index, not merely unreachable",
    );
    assert_eq!(
        rows(&gpu, B_PDB),
        before,
        "★ …and B's rows are untouched, so the count above is not measuring a clear-all",
    );
    // The promotion that named it is refused BY NAME — never routed by a stale row.
    assert_eq!(
        gpu.promote_ctx(&promotion(
            A_CLIENT,
            H_GR_CHANNEL,
            vec![gr_range(GR_VA, 0x2_ef94_6000)]
        )),
        Err(PromoteFault::UnknownContextObject {
            client: A_CLIENT,
            object: H_GR_CHANNEL,
        }),
    );
}

// =================================================================================
// 3. MEAN — through the L1 shell, both lock modes
// =================================================================================

/// ★★ The MEAN integration test: multi-proc × identical guest VAs × the full state
/// cross-product × re-promote of an already-bound buffer × a proc retiring mid-sequence,
/// driven through [`SharedDevice`] so the **two-pass lock discipline** (rank-0 route, then
/// the owner's rank-1 lock alone) is what is exercised — not the single-owner shortcut.
#[test]
fn mean_promote_through_the_shell() {
    for mode in [LockMode::Sharded, LockMode::Degenerate] {
        let gpu = world();
        // The two ProcIds, read off the bare device before it goes behind the lock
        // shell — the shell's own read paths are what the assertions below exercise.
        let (a_pid, b_pid) = (pid_of(&gpu, A_PDB), pid_of(&gpu, B_PDB));
        assert_ne!(a_pid, b_pid);
        let dev = gpu.map(|g| Arc::new(SharedDevice::new(g, mode)));

        // Both procs promote the SAME guest VA, plus a per-proc second buffer, plus the
        // unbindable states arriving as declined counts.
        for (client, phys, second) in [
            (A_CLIENT, 0xA000_0000u64, 0x1_2010_a000u64),
            (B_CLIENT, 0xB000_0000, 0x1_2010_a000),
        ] {
            let join = dev
                .promote_ctx(&CtxPromotion {
                    client,
                    chan_client: client,
                    object: H_GR_CHANNEL,
                    ranges: vec![
                        gr_range(GR_VA, phys),
                        PromotedRange {
                            va: GpuVa(second),
                            len: 0x4000,
                            phys: phys + 0x10_0000,
                            aperture: Aperture::SysmemCoherent,
                            buffer_id: 2,
                        },
                    ],
                    declined: PromoteDeclined {
                        initialize_only: 1,
                        promote_only: 4,
                    },
                })
                .expect("joins");
            assert_eq!((join.bound, join.already), (2, 0));
            assert_eq!(join.declined.promote_only, 4);
        }

        // Re-promote of an already-bound buffer, through the TSG handle this time.
        for client in [A_CLIENT, B_CLIENT] {
            let phys = if client == A_CLIENT {
                0xA000_0000
            } else {
                0xB000_0000
            };
            let again = dev
                .promote_ctx(&promotion(client, H_TSG, vec![gr_range(GR_VA, phys)]))
                .expect("re-promotes");
            assert_eq!((again.bound, again.already), (0, 1));
        }

        // Two distinct tables, neither resolving the other's phys (#14). Read through
        // the shell's own route+resolve, i.e. the same path a doorbell takes.
        let read = |pdb: Pdb, va: GpuVa| {
            dev.resolve(GpuId::ZERO, pdb, va)
                .map(|(b, off)| b.phys + off)
        };
        assert_eq!(read(A_PDB, GR_VA), Ok(0xA000_0000));
        assert_eq!(read(B_PDB, GR_VA), Ok(0xB000_0000));

        // A retires mid-sequence; B is untouched and still promotes.
        assert!(dev.retire_proc(a_pid));
        assert_eq!(read(B_PDB, GR_VA), Ok(0xB000_0000));
        dev.promote_ctx(&promotion(
            B_CLIENT,
            H_GR_CHANNEL,
            vec![PromotedRange {
                va: GpuVa(0x1_2100_0000),
                len: 0x1000,
                phys: 0xB100_0000,
                aperture: Aperture::Vidmem,
                buffer_id: 5,
            }],
        ))
        .expect("B still promotes after A retires");

        // A promotion naming the dead proc's address space is refused by name.
        //
        // ★★★★ §16.40 — and the NAME CHANGED HERE, because this assertion and the comment
        // one line above it disagreed. The comment says *"the dead **proc**'s address
        // space"* — an OWNER fact — while the assertion named `ContextVasUndeclared`,
        // whose documented meaning is *"its VASpace has not declared a page-directory
        // base"*. Both could not be true of one refusal, and the comment was the correct
        // one: `A`'s channel and VASpace are still in the graph, so `Spine::ctx_vas` still
        // answers `(gpu, pdb)`; what retiring `A` removed is the `Spine::by_pdb` entry
        // that names the owner. That is hop 3, and it now refuses under its own name.
        //
        // ⊘ This is `a_comment_that_names_an_exception_is_a_bug_report` in its mildest
        // form: nothing was broken, but the census this test pins fed a boot report where
        // one tag stood for two opposite diagnoses — *"the root never arrived"* and *"the
        // root arrived and the owner index lost it"*. Three rungs read `s35`'s single
        // `ContextVasUndeclared x1` as the first without anything able to refute the
        // second.
        assert_eq!(
            dev.promote_ctx(&promotion(
                A_CLIENT,
                H_GR_CHANNEL,
                vec![gr_range(GR_VA, 0xA000_0000)]
            )),
            Err(PromoteFault::ContextVasNoOwner {
                client: A_CLIENT,
                object: H_GR_CHANNEL,
                pdb: A_PDB,
            }),
            "{mode:?}",
        );
    }
}

// =================================================================================
// 4. End to end — wire bytes through the bridge into the table
// =================================================================================

/// ★★ The whole chain: a real `GSP_RM_CONTROL` message → `translate` →
/// `Translation::CtxPromotion` → `Gpu::promote_ctx` → a resolvable VA.
#[test]
fn wire_bytes_reach_the_address_table() {
    let mut gpu = world();
    let params = w::promote_ctx_params(
        1,
        A_CLIENT.0,
        H_GR_CHANNEL.0,
        3,
        &[
            complete(GR_VA.0, GR_LEN, 0x2_ef94_6000, 0, 0),
            promote_only(0x1_2019_0000, 3),
            initialize_only(0x80000, 0x2_ef82_0000, 0, 10),
        ],
    );
    let msg = w::message(
        fn_id::GSP_RM_CONTROL,
        7,
        &w::control_body(
            A_CLIENT.0,
            0x5c00_00ff, // the Subdevice the control is issued against — dropped
            w::NV2080_CTRL_CMD_GPU_PROMOTE_CTX,
            560,
            w::RMAPI_RPC_FLAGS_NONE,
            &params,
        ),
    );

    let mut policy = GraphPolicy::new(abi(), GuestOs::Linux, &mut gpu);
    let out = policy.deliver(&kayfabe_gsp::RpcCommand {
        function: kayfabe_gsp::RpcFunction::RmControl,
        code: fn_id::GSP_RM_CONTROL,
        sequence: 7,
        payload: abi().rpc_payload(&msg).expect("payload").to_vec(),
        elements: 1,
        delivered: Vec::new(),
    });
    let Ok(Translation::CtxPromotion(p)) = out else {
        panic!("expected a context promotion, got {out:?}");
    };
    // ★ The two clients, both carried, doing their two different jobs.
    assert_eq!(p.client, A_CLIENT, "attribution = the ENVELOPE's hClient");
    assert_eq!(
        p.chan_client, A_CLIENT,
        "resolution = the params' hChanClient",
    );
    assert_eq!(p.object, H_GR_CHANNEL);
    assert_eq!(p.ranges, vec![gr_range(GR_VA, 0x2_ef94_6000)]);
    assert_eq!(
        p.declined,
        PromoteDeclined {
            initialize_only: 1,
            promote_only: 1,
        },
    );
    assert_eq!(policy.promoted(), 1, "★ non-vacuity: the join actually ran");
    assert!(policy.census().is_empty());
    drop(policy);

    assert_eq!(resolve_in(&gpu, A_PDB, GR_VA), Ok(0x2_ef94_6000));
}

/// A guest whose declared `paramsSize` disagrees with the struct is refused with **both**
/// numbers, and nothing is written.
///
/// ★ Only the **under**-declared side reaches this refusal, and the reason is the
/// refusal ORDER rather than a gap: a `paramsSize` larger than the payload that arrived
/// is refused one check earlier, by `ParamsSizeExceedsPayload`, which carries the two
/// numbers that actually matter there. Asserted below so the order is pinned rather than
/// assumed.
#[test]
fn a_declared_size_that_is_not_560_is_refused_with_both_numbers() {
    for declared in [0u32, 32, 559] {
        let params = w::promote_ctx_params(
            1,
            A_CLIENT.0,
            H_GR_CHANNEL.0,
            1,
            &[complete(GR_VA.0, GR_LEN, 0x2_ef94_6000, 0, 0)],
        );
        let msg = w::message(
            fn_id::GSP_RM_CONTROL,
            7,
            &w::control_body(
                A_CLIENT.0,
                0x5c00_00ff,
                w::NV2080_CTRL_CMD_GPU_PROMOTE_CTX,
                declared,
                w::RMAPI_RPC_FLAGS_NONE,
                &params,
            ),
        );
        let got = kayfabe_rmrpc::translate(
            abi(),
            GuestOs::Linux,
            &kayfabe_gsp::RpcCommand {
                function: kayfabe_gsp::RpcFunction::RmControl,
                code: fn_id::GSP_RM_CONTROL,
                sequence: 7,
                payload: abi().rpc_payload(&msg).expect("payload").to_vec(),
                elements: 1,
                delivered: Vec::new(),
            },
        );
        assert_eq!(
            got,
            Err(BridgeRefusal::ControlParamsSizeMismatch {
                cmd: 0x2080_012b,
                declared,
                expected: 560,
            }),
        );
    }
    // The over-declared side, refused one check earlier and with its own two numbers.
    for declared in [561u32, 4096] {
        let params = w::promote_ctx_params(
            1,
            A_CLIENT.0,
            H_GR_CHANNEL.0,
            1,
            &[complete(GR_VA.0, GR_LEN, 0x2_ef94_6000, 0, 0)],
        );
        let msg = w::message(
            fn_id::GSP_RM_CONTROL,
            7,
            &w::control_body(
                A_CLIENT.0,
                0x5c00_00ff,
                w::NV2080_CTRL_CMD_GPU_PROMOTE_CTX,
                declared,
                w::RMAPI_RPC_FLAGS_NONE,
                &params,
            ),
        );
        assert_eq!(
            kayfabe_rmrpc::translate(
                abi(),
                GuestOs::Linux,
                &kayfabe_gsp::RpcCommand {
                    function: kayfabe_gsp::RpcFunction::RmControl,
                    code: fn_id::GSP_RM_CONTROL,
                    sequence: 7,
                    payload: abi().rpc_payload(&msg).expect("payload").to_vec(),
                    elements: 1,
                    delivered: Vec::new(),
                }
            ),
            Err(BridgeRefusal::ParamsSizeExceedsPayload {
                declared,
                available: 560,
            }),
        );
    }
    // ★★ And the case that makes "exact" load-bearing rather than decorative: a params
    // blob that is LONGER than the struct, declared at its own length. The payload bound
    // is satisfied and the decoder accepts a long buffer, so a `declared < expected`
    // check would let it through — decoding a 600-byte struct the guest meant as
    // something else. `gsp_core_bridge.md` §4.3: refuse the mismatch, never resolve it
    // in either direction.
    let mut over = w::promote_ctx_params(
        1,
        A_CLIENT.0,
        H_GR_CHANNEL.0,
        1,
        &[complete(GR_VA.0, GR_LEN, 0x2_ef94_6000, 0, 0)],
    );
    over.extend_from_slice(&[0u8; 40]);
    let msg = w::message(
        fn_id::GSP_RM_CONTROL,
        7,
        &w::control_body(
            A_CLIENT.0,
            0x5c00_00ff,
            w::NV2080_CTRL_CMD_GPU_PROMOTE_CTX,
            600,
            w::RMAPI_RPC_FLAGS_NONE,
            &over,
        ),
    );
    assert_eq!(
        kayfabe_rmrpc::translate(
            abi(),
            GuestOs::Linux,
            &kayfabe_gsp::RpcCommand {
                function: kayfabe_gsp::RpcFunction::RmControl,
                code: fn_id::GSP_RM_CONTROL,
                sequence: 7,
                payload: abi().rpc_payload(&msg).expect("payload").to_vec(),
                elements: 1,
                delivered: Vec::new(),
            }
        ),
        Err(BridgeRefusal::ControlParamsSizeMismatch {
            cmd: 0x2080_012b,
            declared: 600,
            expected: 560,
        }),
    );
}

/// ★★ **C defect D7.** The C's reply builder had no case for `PROMOTE_CTX`, so it fell
/// into the generic replay and overwrote the guest's own params with a **hard-coded
/// capture from a different machine and a different boot** — stale handles, stale VAs,
/// stale framebuffer physicals — under `NV_OK`. The guest's CPU-RM then read
/// `promoteEntry[i].bInitialize` back out of that foreign blob to decide which buffers to
/// mark initialized, so the clobber fed guest *state*, not just guest memory.
///
/// Here a Case-2 ACK writes back **nothing**.
#[test]
fn a_case2_ack_writes_nothing_back() {
    let gpu = world();
    let dev = gpu.map(|g| Arc::new(SharedDevice::new(g, LockMode::Sharded)));
    let mut payload = w::promote_ctx_params(
        1,
        A_CLIENT.0,
        H_GR_CHANNEL.0,
        1,
        &[complete(GR_VA.0, GR_LEN, 0x2_ef94_6000, 0, 0)],
    );
    let before = payload.clone();
    let route = dev
        .route_control(
            GpuId::ZERO,
            kayfabe_core::gpu::Gpu::SYSTEM_PROC,
            kayfabe_isolate::HostHandle::new(kayfabe_isolate::IsolateId::new(0, GpuId::ZERO), 1),
            mock_ctrl::PROMOTE_CTX,
            &mut payload,
        )
        .expect("Case-2 is ACKed");
    assert_eq!(route, kayfabe_fwd::ControlRoute::AckOnly);
    assert_eq!(
        payload, before,
        "★ D7: not one byte of the caller's buffer may change",
    );
    assert_eq!(
        ControlCmd(0x2080_012b),
        mock_ctrl::PROMOTE_CTX,
        "the Case-2 command really is the one under test",
    );
}
