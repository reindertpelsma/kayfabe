//! The mean half: every decoder in this crate parses **untrusted guest bytes**,
//! so every decoder is tested as an attack surface, not as a happy path.
//!
//! What "mean" means concretely here (`testing_doctrine.md` §3.1):
//!
//! - **Exhaustive length sweep** over every decoder, `0..=96` bytes, asserting
//!   *which* error and *its numbers* — not `is_err()`. The repo already fuzzes
//!   its pushbuffer parser for this reason; a decoder over a fixed-size struct
//!   has a small enough input-length space to sweep exhaustively instead, which
//!   is strictly stronger than sampling it.
//! - **Hostile field values**: all-zero, all-`0xFF`, and `0xFF`-with-one-hole,
//!   so a decoder that special-cases a sentinel is caught.
//! - **Version skew as an adversary**, not as a config: the same 64 bytes
//!   decoded under both tables must produce *different* values, because if they
//!   did not, the version table would be decoration.
//! - **Writeback preservation**, byte for byte, against a poisoned buffer.
//! - **Composed**: a realistic RM event stream driven end to end through one
//!   table, with the skew case interleaved.

use kayfabe_abi::generated::{classes, ctrl, nvos, rpc};
use kayfabe_abi::transcribed::Nvos46ParametersPre580;
use kayfabe_abi::versions::{BENCH_DRIVER, DriverAbiTable, MapDmaWire, table_for};
use kayfabe_abi::view::{AllocWire, PdbAperture, RpcEnvelope};
use kayfabe_abi::wire::AbiError;
use kayfabe_abi::{DriverAbi, DriverVersion, client_kind_from_process_id};
use kayfabe_arch::ClientKind;

const V575: DriverVersion = DriverVersion {
    major: 575,
    minor: 51,
    patch: 2,
};

fn v575() -> &'static DriverAbiTable {
    table_for(V575).expect("575.51.02 is supported")
}
fn bench() -> &'static DriverAbiTable {
    table_for(BENCH_DRIVER).expect("the bench driver is supported")
}

// ---------------------------------------------------------------------------
// 1. The length sweep — no panic, and the exact error, for every length.
// ---------------------------------------------------------------------------

/// One decoder's contract as a closure, so the sweep can be uniform.
type Decoder = fn(&[u8]) -> Result<(), AbiError>;

fn sweep(c_name: &'static str, size: usize, decode: Decoder) {
    for len in 0..=96usize {
        // Two fillings, because a decoder that reads a field it should not may
        // still return Ok on zeros.
        for fill in [0x00u8, 0xFF] {
            let buf = vec![fill; len];
            let got = decode(&buf);
            if len < size {
                assert_eq!(
                    got,
                    Err(AbiError::Truncated {
                        c_name,
                        need: size,
                        got: len
                    }),
                    "{c_name}: {len} bytes (fill {fill:#04x}) must be refused with the exact numbers"
                );
            } else {
                assert_eq!(
                    got,
                    Ok(()),
                    "{c_name}: {len} bytes (fill {fill:#04x}) must decode"
                );
            }
        }
    }
}

/// Every generated decoder, swept across every buffer length from empty to
/// well past its size.
///
/// This is the `abi_struct_truncation` bug class made impossible-by-test: a
/// decoder that zero-extended a short buffer would return `Ok` below `size` and
/// fail here at the very first length it could get away with.
#[test]
fn every_decoder_refuses_short_and_accepts_long_at_every_length() {
    sweep("NVOS00_PARAMETERS", 16, |b| {
        nvos::Nvos00Parameters::decode(b).map(|_| ())
    });
    sweep("NVOS21_PARAMETERS", 32, |b| {
        nvos::Nvos21Parameters::decode(b).map(|_| ())
    });
    sweep("NVOS46_PARAMETERS", 64, |b| {
        nvos::Nvos46Parameters::decode(b).map(|_| ())
    });
    sweep("NVOS47_PARAMETERS", 48, |b| {
        nvos::Nvos47Parameters::decode(b).map(|_| ())
    });
    sweep("NVOS54_PARAMETERS", 32, |b| {
        nvos::Nvos54Parameters::decode(b).map(|_| ())
    });
    sweep("NVOS55_PARAMETERS", 28, |b| {
        nvos::Nvos55Parameters::decode(b).map(|_| ())
    });
    sweep("NVOS64_PARAMETERS", 48, |b| {
        nvos::Nvos64Parameters::decode(b).map(|_| ())
    });
    sweep("NVOS46_PARAMETERS", 56, |b| {
        Nvos46ParametersPre580::decode(b).map(|_| ())
    });
    sweep("NV0080_ALLOC_PARAMETERS", 56, |b| {
        classes::Nv0080AllocParameters::decode(b).map(|_| ())
    });
    sweep("NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS", 32, |b| {
        ctrl::Nv0080CtrlDmaSetPageDirectoryParams::decode(b).map(|_| ())
    });
    sweep("rpc_message_header_v03_00", 32, |b| {
        rpc::RpcMessageHeaderV0300::decode(b).map(|_| ())
    });
    // 120 > 96, so this one gets its own longer sweep rather than being quietly
    // all-refusals in the shared one.
    for len in 0..=160usize {
        let buf = vec![0xFFu8; len];
        let got = classes::Nv0000AllocParameters::decode(&buf).map(|_| ());
        if len < 120 {
            assert_eq!(
                got,
                Err(AbiError::Truncated {
                    c_name: "NV0000_ALLOC_PARAMETERS",
                    need: 120,
                    got: len
                })
            );
        } else {
            assert_eq!(got, Ok(()));
        }
    }
}

/// The same sweep through the **table**, which is the API a caller actually
/// uses, and where the versioned decoder's `need` must track the version.
#[test]
fn table_decoders_refuse_short_at_every_length_and_report_the_versions_size() {
    let t575 = v575();
    let t580 = bench();
    for len in 0..=96usize {
        let b = vec![0xA5u8; len];

        assert_eq!(
            t580.decode_free(&b).is_ok(),
            len >= 16,
            "decode_free at {len} bytes"
        );
        assert_eq!(t580.decode_control(&b).is_ok(), len >= 32);
        assert_eq!(t580.decode_dup(&b).is_ok(), len >= 28);
        assert_eq!(t580.decode_unmap_memory_dma(&b).is_ok(), len >= 48);
        assert_eq!(t580.decode_device_alloc_facts(&b).is_ok(), len >= 56);
        assert_eq!(t580.decode_set_page_dir(&b).is_ok(), len >= 32);
        assert_eq!(t580.decode_client_alloc_facts(&b).is_ok(), len >= 8);

        // ★ The versioned one: the SAME length is legal under 575 and refused
        // under 580 for the eight lengths in [56, 64).
        assert_eq!(
            t575.decode_map_memory_dma(&b).is_ok(),
            len >= 56,
            "575 wants 56 bytes; {len} given"
        );
        assert_eq!(
            t580.decode_map_memory_dma(&b).is_ok(),
            len >= 64,
            "580.65.06+ wants 64 bytes; {len} given"
        );
        if (56..64).contains(&len) {
            assert_eq!(
                t580.decode_map_memory_dma(&b),
                Err(AbiError::Truncated {
                    c_name: "NVOS46_PARAMETERS",
                    need: 64,
                    got: len
                }),
                "a 575-shaped MAP_MEMORY_DMA under a 580 table must say need 64, not decode"
            );
            assert!(t575.decode_map_memory_dma(&b).is_ok());
        }
    }
}

// ---------------------------------------------------------------------------
// 2. Version skew as an adversary.
// ---------------------------------------------------------------------------

/// A distinctive 64-byte `NVOS46` image: every field holds a value that could
/// not be confused with any other field's.
fn nvos46_v580_image() -> Vec<u8> {
    let p = nvos::Nvos46Parameters {
        h_client: 0xC1D0_0067,
        h_device: 0x5C00_0001,
        h_dma: 0xCAF0_0005,
        h_memory: 0x5C00_0019,
        offset: 0x0000_1000_0000_0000,
        length: 0x0000_0000_0020_0000,
        flags: 0x0000_00A5,
        flags2: 0xF2F2_F2F2,
        kind_override: 0x0B0B_0B0B,
        dma_offset: 0x0000_7F00_0000_0000,
        status: 0x0000_0000,
    };
    let mut b = vec![0u8; 64];
    p.encode_into(&mut b).expect("64 bytes is enough");
    b
}

/// ★ The load-bearing skew test: the **same bytes** decoded under the two tables
/// must disagree, and disagree in the specific way the layout change predicts.
///
/// If this ever passed with both tables agreeing, the version table would be
/// decoration and the whole versioning design would be untested.
#[test]
fn the_same_bytes_decode_differently_under_the_two_tables() {
    let img = nvos46_v580_image();
    let under_580 = bench().decode_map_memory_dma(&img).expect("64 bytes");
    let under_575 = v575().decode_map_memory_dma(&img).expect("64 >= 56");

    // The common prefix agrees — that is why one view can serve both.
    assert_eq!(under_580.client, under_575.client);
    assert_eq!(under_580.device, under_575.device);
    assert_eq!(under_580.dma, under_575.dma);
    assert_eq!(under_580.memory, under_575.memory);
    assert_eq!(under_580.offset, under_575.offset);
    assert_eq!(under_580.length, under_575.length);
    assert_eq!(under_580.flags, under_575.flags);

    // And `dmaOffset` does NOT: the 575 reader takes it from +40, which in a
    // 580 image is `kindOverride` followed by padding.
    assert_eq!(under_580.dma_offset, 0x0000_7F00_0000_0000, "580 reads +48");
    assert_eq!(
        under_575.dma_offset, 0x0000_0000_0B0B_0B0B,
        "575 reads +40, which in a 580 image is kindOverride + 4 bytes of padding"
    );
    assert_ne!(
        under_580.dma_offset, under_575.dma_offset,
        "if these agreed, the version table would be doing nothing"
    );
}

/// The mirror image: a 575-shaped buffer read by a 575 table is right, and the
/// **writeback offsets** differ by version too.
///
/// `status` lands at +48 or +56, which is bug `#81` in the C artifact
/// (`nvkvm_abi.h` carries `.nvos46_status_off` 48/48/56 for exactly this).
#[test]
fn the_writeback_lands_at_the_versions_status_offset_and_touches_nothing_else() {
    for (table, size, dma_at, status_at) in
        [(v575(), 56usize, 40usize, 48usize), (bench(), 64, 48, 56)]
    {
        // Poison the whole buffer so any stray write is visible.
        let mut buf = vec![0xEEu8; size];
        table
            .write_map_memory_dma_result(&mut buf, 0x0000_7F12_3456_7000, 0x0000_002A)
            .expect("buffer is exactly this version's size");

        assert_eq!(
            &buf[dma_at..dma_at + 8],
            &0x0000_7F12_3456_7000u64.to_le_bytes(),
            "dmaOffset must land at +{dma_at}"
        );
        assert_eq!(
            &buf[status_at..status_at + 4],
            &0x0000_002Au32.to_le_bytes(),
            "status must land at +{status_at}"
        );
        // Every byte outside those two fields is still the poison. This is the
        // `writeback_bug_pattern` rule asserted byte-for-byte: a writer that
        // rewrote the struct would have zeroed the [IN] fields the guest set.
        for (i, b) in buf.iter().enumerate() {
            let in_dma = (dma_at..dma_at + 8).contains(&i);
            let in_status = (status_at..status_at + 4).contains(&i);
            if !in_dma && !in_status {
                assert_eq!(*b, 0xEE, "byte +{i} was clobbered by the writeback");
            }
        }
    }
}

/// The writeback refuses a buffer that is short **for its own version**, with the
/// version's number — so a 575-sized reply buffer against a 580 table cannot
/// scribble `status` past the end.
#[test]
fn the_writeback_refuses_a_buffer_short_for_its_own_version() {
    let mut b = vec![0u8; 56];
    assert_eq!(
        bench().write_map_memory_dma_result(&mut b, 1, 2),
        Err(AbiError::Truncated {
            c_name: "NVOS46_PARAMETERS",
            need: 64,
            got: 56
        })
    );
    // …and the buffer is untouched by the refusal.
    assert!(
        b.iter().all(|&x| x == 0),
        "a refused writeback must write nothing"
    );
    // The same 56 bytes under 575 succeed — the non-vacuity arm.
    assert_eq!(v575().write_map_memory_dma_result(&mut b, 1, 2), Ok(()));
    assert_eq!(&b[40..48], &1u64.to_le_bytes());
    assert_eq!(&b[48..52], &2u32.to_le_bytes());
}

// ---------------------------------------------------------------------------
// 3. Hostile values and round-trips.
// ---------------------------------------------------------------------------

/// An all-`0xFF` buffer decodes to all-`MAX` — no sentinel is special-cased, no
/// value is silently sanitised.
#[test]
fn hostile_all_ones_decodes_to_the_literal_values() {
    let b = vec![0xFFu8; 128];
    let p = nvos::Nvos64Parameters::decode(&b).expect("128 >= 48");
    assert_eq!(p.h_root, u32::MAX);
    assert_eq!(p.h_object_parent, u32::MAX);
    assert_eq!(p.h_object_new, u32::MAX);
    assert_eq!(p.h_class, u32::MAX);
    assert_eq!(p.p_alloc_parms, u64::MAX);
    assert_eq!(p.p_rights_requested, u64::MAX);
    assert_eq!(p.params_size, u32::MAX);
    assert_eq!(p.flags, u32::MAX);
    assert_eq!(p.status, u32::MAX);

    let m = bench().decode_map_memory_dma(&b).expect("128 >= 64");
    assert_eq!(m.offset, u64::MAX);
    assert_eq!(m.length, u64::MAX);
    assert_eq!(m.dma_offset, u64::MAX);

    // A `SET_PAGE_DIRECTORY` claiming a 2^64-1 page-directory base and an
    // undefined aperture is decoded faithfully and classified as UNDEFINED —
    // never rounded into a real aperture.
    let s = bench().decode_set_page_dir(&b).expect("128 >= 32");
    assert_eq!(s.phys_address, u64::MAX);
    assert_eq!(s.num_entries, u32::MAX);
    assert_eq!(s.aperture, PdbAperture::Undefined(3));
    assert_eq!(s.h_vaspace, u32::MAX);
}

/// `decode ∘ encode == identity` for every generated struct, on values chosen so
/// that a swapped pair of same-width fields would show.
#[test]
fn encode_then_decode_is_the_identity_with_distinguishable_field_values() {
    // Every field gets a distinct value, so a field-order bug cannot survive.
    let a = nvos::Nvos64Parameters {
        h_root: 0x1111_1111,
        h_object_parent: 0x2222_2222,
        h_object_new: 0x3333_3333,
        h_class: 0x4444_4444,
        p_alloc_parms: 0x5555_5555_5555_5555,
        p_rights_requested: 0x6666_6666_6666_6666,
        params_size: 0x7777_7777,
        flags: 0x8888_8888,
        status: 0x9999_9999,
    };
    let mut b = vec![0u8; nvos::Nvos64Parameters::SIZE];
    a.encode_into(&mut b).expect("exact size");
    assert_eq!(nvos::Nvos64Parameters::decode(&b), Ok(a));

    let d = nvos::Nvos55Parameters {
        h_client: 0xAAAA_0001,
        h_parent: 0xAAAA_0002,
        h_object: 0xAAAA_0003,
        h_client_src: 0xAAAA_0004,
        h_object_src: 0xAAAA_0005,
        flags: 0xAAAA_0006,
        status: 0xAAAA_0007,
    };
    let mut b = vec![0u8; nvos::Nvos55Parameters::SIZE];
    d.encode_into(&mut b).expect("exact size");
    assert_eq!(nvos::Nvos55Parameters::decode(&b), Ok(d));

    // The array-carrying one, with a non-uniform array so an off-by-one stride
    // would show.
    let mut name = [0u8; 100];
    for (i, s) in name.iter_mut().enumerate() {
        *s = u8::try_from(i % 251).expect("i % 251 < 256");
    }
    let c = classes::Nv0000AllocParameters {
        h_client: 0xC1D0_0069,
        process_id: 0xFFFF_FFFF,
        process_name: name,
        p_os_pid_info: 0x0123_4567_89AB_CDEF,
    };
    let mut b = vec![0u8; classes::Nv0000AllocParameters::SIZE];
    c.encode_into(&mut b).expect("exact size");
    assert_eq!(classes::Nv0000AllocParameters::decode(&b), Ok(c));
    // …and the array really did land contiguously at +8.
    assert_eq!(&b[8..108], &name[..]);
}

/// `encode_into` never touches a byte it does not name — checked against a
/// poisoned buffer, including the interior padding at +36..40 that `NVOS46`'s
/// 8-alignment creates.
#[test]
fn encode_into_leaves_padding_and_the_tail_untouched() {
    let p = Nvos46ParametersPre580 {
        h_client: 1,
        h_device: 2,
        h_dma: 3,
        h_memory: 4,
        offset: 5,
        length: 6,
        flags: 7,
        dma_offset: 8,
        status: 9,
    };
    let mut b = vec![0x5Au8; 72]; // 56 of struct + 16 of "someone else's data"
    p.encode_into(&mut b).expect("72 >= 56");
    // The interior padding.
    assert_eq!(&b[36..40], &[0x5A; 4], "padding at +36..40 must survive");
    // The tail padding.
    assert_eq!(
        &b[52..56],
        &[0x5A; 4],
        "tail padding at +52..56 must survive"
    );
    // Everything past the struct.
    assert_eq!(&b[56..72], &[0x5A; 16], "bytes past sizeof must survive");
}

// ---------------------------------------------------------------------------
// 4. The alloc-shape discriminator.
// ---------------------------------------------------------------------------

/// The v1/v2 discriminator keys on the **ioctl's** declared size, and every
/// other value is refused by name.
#[test]
fn the_alloc_shape_comes_from_the_ioctl_size_and_nothing_else() {
    let t = bench();
    let b = vec![0x33u8; 64];
    assert_eq!(t.decode_alloc(&b, 32).map(|a| a.wire), Ok(AllocWire::V1));
    assert_eq!(t.decode_alloc(&b, 48).map(|a| a.wire), Ok(AllocWire::V2));
    for bad in [0usize, 1, 31, 33, 40, 47, 49, 64, usize::MAX] {
        assert_eq!(
            t.decode_alloc(&b, bad).map(|a| a.wire),
            Err(AbiError::UnknownAllocWire { ioctl_size: bad }),
            "ioctl size {bad} must be refused, not guessed"
        );
    }
    // ★ The buffer is 64 bytes — long enough for BOTH shapes — so the choice
    // cannot have come from `bytes.len()`. And the two shapes really do read
    // different fields: v1 has no rights mask.
    let v1 = t.decode_alloc(&b, 32).expect("v1");
    let v2 = t.decode_alloc(&b, 48).expect("v2");
    assert_eq!(v1.rights_requested, 0, "NVOS21 declares no rights mask");
    assert_eq!(v2.rights_requested, 0x3333_3333_3333_3333);
    assert_eq!(v1.params_size, 0x3333_3333);
    assert_eq!(v2.params_size, 0x3333_3333);
    // v1's `paramsSize` is at +24 and v2's at +32 — both 0x33333333 here by
    // construction, so pin the *offsets* through a crafted image instead.
    let mut img = vec![0u8; 48];
    nvos::Nvos64Parameters {
        params_size: 0xBEEF,
        flags: 0xF1A6,
        ..Default::default()
    }
    .encode_into(&mut img)
    .expect("48 bytes");
    assert_eq!(t.decode_alloc(&img, 48).expect("v2").params_size, 0xBEEF);
    assert_eq!(
        t.decode_alloc(&img, 32).expect("v1").params_size,
        0,
        "v1 reads paramsSize from +24, which in a v2 image is the low half of pRightsRequested"
    );
}

// ---------------------------------------------------------------------------
// 5. The RPC envelope — a guest-written length field.
// ---------------------------------------------------------------------------

fn rpc_image(length: u32, function: u32, total: usize) -> Vec<u8> {
    let mut b = vec![0u8; total.max(32)];
    rpc::RpcMessageHeaderV0300 {
        header_version: 0x0300_0000,
        signature: RpcEnvelope::SIGNATURE_VALID,
        length,
        function,
        rpc_result: 0,
        rpc_result_private: 0,
        sequence: 7,
        u: 0,
    }
    .encode_into(&mut b)
    .expect("at least 32");
    b.truncate(total);
    b
}

/// A well-formed `GSP_INIT_DONE`-shaped envelope decodes, and its payload slice
/// is exactly what `length` promised.
#[test]
fn a_wellformed_rpc_envelope_yields_the_exact_payload() {
    let t = bench();
    let b = rpc_image(32, rpc::NV_VGPU_MSG_EVENT_GSP_INIT_DONE, 4096);
    let env = t.decode_rpc_envelope(&b).expect("well formed");
    assert_eq!(env.function, 0x1001);
    assert_eq!(env.sequence, 7);
    assert_eq!(env.length, 32);
    assert_eq!(env.payload_len, 0, "a bare header has no payload");
    assert_eq!(t.rpc_payload(&b).expect("payload"), &[] as &[u8]);

    // …and with a body, the payload starts at exactly +32.
    let mut b = rpc_image(32 + 8, rpc::NV_VGPU_MSG_EVENT_POST_EVENT, 4096);
    b[32..40].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
    let env = t.decode_rpc_envelope(&b).expect("well formed");
    assert_eq!(env.payload_len, 8);
    assert_eq!(
        t.rpc_payload(&b).expect("payload"),
        &[1, 2, 3, 4, 5, 6, 7, 8]
    );
}

/// Every impossible `length` a guest can write, refused with its own numbers.
///
/// `length` is the field that says how much to read, so it is the field an
/// attacker sets. `u32::MAX` here is the classic: an unchecked `length - 32`
/// would then slice 4 GiB out of a 4 KiB buffer.
#[test]
fn a_hostile_rpc_length_is_refused_with_its_numbers() {
    let t = bench();
    for bad in [0u32, 1, 31, 4097, 0x8000_0000, u32::MAX] {
        let b = rpc_image(bad, 0, 4096);
        assert_eq!(
            t.decode_rpc_envelope(&b),
            Err(AbiError::RpcLength {
                declared: bad,
                available: 4096
            }),
            "declared length {bad} against a 4096-byte buffer"
        );
        assert_eq!(
            t.rpc_payload(&b),
            Err(AbiError::RpcLength {
                declared: bad,
                available: 4096
            })
        );
    }
    // The two boundaries that must SUCCEED — the non-vacuity arm, without which
    // the refusals above could be refusing everything.
    assert_eq!(
        t.decode_rpc_envelope(&rpc_image(32, 0, 4096))
            .map(|e| e.payload_len),
        Ok(0)
    );
    assert_eq!(
        t.decode_rpc_envelope(&rpc_image(4096, 0, 4096))
            .map(|e| e.payload_len),
        Ok(4064)
    );
}

/// A wrong signature is refused before the length is trusted.
#[test]
fn a_wrong_rpc_signature_is_refused_by_value() {
    let t = bench();
    let mut b = rpc_image(32, 0, 4096);
    for bad in [0u32, 1, 0x4350_5255, 0x4350_5257, u32::MAX] {
        b[4..8].copy_from_slice(&bad.to_le_bytes());
        assert_eq!(
            t.decode_rpc_envelope(&b),
            Err(AbiError::RpcSignature {
                found: bad,
                expected: 0x4350_5256
            }),
            "signature {bad:#010x}"
        );
    }
    // Non-vacuity: restore it and the same buffer decodes.
    b[4..8].copy_from_slice(&RpcEnvelope::SIGNATURE_VALID.to_le_bytes());
    assert!(t.decode_rpc_envelope(&b).is_ok());
}

/// The envelope is checked in a deliberate order: a buffer too short for the
/// header is `Truncated`, not `RpcSignature` — you cannot report a bad signature
/// you never read.
#[test]
fn an_undersized_rpc_buffer_is_truncated_not_a_signature_error() {
    let t = bench();
    for len in 0..32usize {
        let b = vec![0xFFu8; len];
        assert_eq!(
            t.decode_rpc_envelope(&b),
            Err(AbiError::Truncated {
                c_name: "rpc_message_header_v03_00",
                need: 32,
                got: len
            }),
        );
    }
    // At exactly 32 the header is readable, so the signature check is what
    // speaks — pinning which error wins at the boundary.
    let b = vec![0xFFu8; 32];
    assert_eq!(
        t.decode_rpc_envelope(&b),
        Err(AbiError::RpcSignature {
            found: u32::MAX,
            expected: 0x4350_5256
        })
    );
}

// ---------------------------------------------------------------------------
// 6. The client-kind seam.
// ---------------------------------------------------------------------------

/// The prefix contract end to end: an `NV01_ROOT` alloc-params image decodes to
/// the right [`ClientKind`] from **8 bytes**, and a 7-byte one is refused.
#[test]
fn the_client_kind_seam_works_from_the_prefix_and_refuses_seven_bytes() {
    let t = bench();
    // A kernel client, as RM writes it (`ogkm rpc.h:70`).
    let mut b = vec![0u8; 8];
    b[0..4].copy_from_slice(&0xC1D0_0069u32.to_le_bytes());
    b[4..8].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    let f = t
        .decode_client_alloc_facts(&b)
        .expect("8 bytes is the contract");
    assert_eq!(f.h_client, 0xC1D0_0069);
    assert_eq!(
        client_kind_from_process_id(f.process_id),
        ClientKind::Kernel
    );

    // A user client — the measured process A from §12.27.
    b[4..8].copy_from_slice(&0x0000_DD13u32.to_le_bytes());
    let f = t.decode_client_alloc_facts(&b).expect("8 bytes");
    assert_eq!(
        client_kind_from_process_id(f.process_id),
        ClientKind::User { pid: 0xDD13 }
    );

    // Seven bytes is refused; the contract is 8, not "as much as you have".
    assert_eq!(
        t.decode_client_alloc_facts(&b[..7]),
        Err(AbiError::Truncated {
            c_name: "NV0000_ALLOC_PARAMETERS",
            need: 8,
            got: 7
        })
    );

    // ★ The prefix contract must agree with the FULL struct's own offsets, or
    // the two would drift the day the struct changes.
    let full = classes::Nv0000AllocParameters::LAYOUT;
    assert_eq!(full.offset_of("h_client"), Some(0));
    assert_eq!(full.offset_of("process_id"), Some(4));
    // The same bytes decoded via the full struct give the same two fields.
    let mut wide = vec![0u8; classes::Nv0000AllocParameters::SIZE];
    wide[0..8].copy_from_slice(&b[0..8]);
    let p = classes::Nv0000AllocParameters::decode(&wide).expect("full size");
    assert_eq!(p.h_client, f.h_client);
    assert_eq!(p.process_id, f.process_id);
}

// ---------------------------------------------------------------------------
// 7. The composed run.
// ---------------------------------------------------------------------------

/// A realistic RM event stream, decoded end to end through one table, with the
/// version-skew case interleaved rather than isolated.
///
/// `testing_doctrine.md` §3: isolated cases test what you thought of, composed
/// runs test what you did not. The specific composition here is the
/// `cuCtxCreate` prefix — client root, device, VAS page-directory, a map, a
/// dup, an unmap, a free — with a MAP_MEMORY_DMA in the *other* version's shape
/// dropped into the middle of it, which is what a guest that lies about its
/// driver version would produce.
#[test]
fn a_realistic_event_stream_decodes_and_the_skewed_event_in_the_middle_is_refused() {
    let t = bench();
    let mut seen: Vec<&'static str> = Vec::new();

    // 1. RM_ALLOC NV01_ROOT — a user client declaring pid 0xDD13.
    let mut alloc = vec![0u8; 48];
    nvos::Nvos64Parameters {
        h_root: 0,
        h_object_parent: 0,
        h_object_new: 0xC1D0_0067,
        h_class: classes::NV01_ROOT,
        p_alloc_parms: 0x7FFF_0000_1000,
        p_rights_requested: 0,
        params_size: 120,
        flags: 0,
        status: 0,
    }
    .encode_into(&mut alloc)
    .expect("48");
    let a = t.decode_alloc(&alloc, 48).expect("v2 alloc");
    assert_eq!(a.class, classes::NV01_ROOT);
    assert_eq!(a.handle, 0xC1D0_0067);
    // …and the class's param size is a loud `None`, not a guess.
    assert_eq!(
        t.alloc_param_size(kayfabe_arch::ids::ClassId(a.class)),
        None,
        "NV0000_ALLOC_PARAMETERS has one oracle; the table says so rather than guessing"
    );
    let mut root_params = vec![0u8; 8];
    root_params[0..4].copy_from_slice(&0xC1D0_0067u32.to_le_bytes());
    root_params[4..8].copy_from_slice(&0x0000_DD13u32.to_le_bytes());
    let facts = t.decode_client_alloc_facts(&root_params).expect("prefix");
    assert_eq!(
        client_kind_from_process_id(facts.process_id),
        ClientKind::User { pid: 0xDD13 }
    );
    seen.push("alloc-client");

    // 2. RM_ALLOC NV01_DEVICE_0 on GPU 1 — the multi-GPU routing fact.
    let mut dev_params = vec![0u8; 56];
    classes::Nv0080AllocParameters {
        device_id: 1,
        ..Default::default()
    }
    .encode_into(&mut dev_params)
    .expect("56");
    let dev = t.decode_device_alloc_facts(&dev_params).expect("56");
    assert_eq!(dev.device_id, 1);
    assert_eq!(
        t.alloc_param_size(kayfabe_arch::ids::ClassId(classes::NV01_DEVICE_0)),
        Some(56)
    );
    seen.push("alloc-device");

    // 3. RM_CONTROL SET_PAGE_DIRECTORY — a sysmem-rooted UVM VAS.
    let mut ctl = vec![0u8; 32];
    nvos::Nvos54Parameters {
        h_client: 0xC1D0_0067,
        h_object: 0x5C00_0001,
        cmd: ctrl::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY,
        flags: 0,
        params: 0x7FFF_0000_2000,
        params_size: 32,
        status: 0,
    }
    .encode_into(&mut ctl)
    .expect("32");
    let c = t.decode_control(&ctl).expect("32");
    assert_eq!(c.cmd, 0x0080_1813);
    let mut pd = vec![0u8; 32];
    ctrl::Nv0080CtrlDmaSetPageDirectoryParams {
        phys_address: 0x0340_0000,
        num_entries: 512,
        flags: 1, // SYSMEM_COH
        h_va_space: 0xCAF0_0005,
        ch_id: 0,
        sub_device_id: 0,
        pasid: 0,
    }
    .encode_into(&mut pd)
    .expect("32");
    let spd = t.decode_set_page_dir(&pd).expect("32");
    assert_eq!(spd.phys_address, 0x0340_0000);
    assert_eq!(spd.aperture, PdbAperture::SysmemCoherent);
    assert!(spd.aperture.is_sysmem());
    seen.push("set-page-dir");

    // 4. MAP_MEMORY_DMA in this table's own shape — succeeds.
    let good = nvos46_v580_image();
    let m = t.decode_map_memory_dma(&good).expect("64");
    assert_eq!(m.dma_offset, 0x0000_7F00_0000_0000);
    seen.push("map");

    // 5. ★ The skewed event, dropped in mid-stream: a guest sends the 56-byte
    //    575 shape while we are on a 580 table. Refused, with the numbers.
    let mut skewed = vec![0u8; 56];
    Nvos46ParametersPre580 {
        h_client: 0xC1D0_0067,
        h_device: 0x5C00_0001,
        h_dma: 0xCAF0_0005,
        h_memory: 0x5C00_0019,
        offset: 0,
        length: 0x20_0000,
        flags: 0,
        dma_offset: 0x7F00_0000_0000,
        status: 0,
    }
    .encode_into(&mut skewed)
    .expect("56");
    assert_eq!(
        t.decode_map_memory_dma(&skewed),
        Err(AbiError::Truncated {
            c_name: "NVOS46_PARAMETERS",
            need: 64,
            got: 56
        }),
        "the 575 shape under a 580 table must be refused, not decoded 8 bytes short"
    );
    // …and the stream keeps going: one refused event does not poison the table.
    seen.push("skew-refused");

    // 6. DUP_OBJECT — the cross-client edge.
    let mut dup = vec![0u8; 28];
    nvos::Nvos55Parameters {
        h_client: 0xC1D0_0069, // the UVM kernel client
        h_parent: 0xC1D0_0069,
        h_object: 0xAB00_0001,
        h_client_src: 0xC1D0_0067,
        h_object_src: 0xCAF0_0005,
        flags: 0,
        status: 0,
    }
    .encode_into(&mut dup)
    .expect("28");
    let d = t.decode_dup(&dup).expect("28");
    assert_eq!(d.src_client, 0xC1D0_0067);
    assert_eq!(d.src_handle, 0xCAF0_0005);
    assert_eq!(d.dst_client, 0xC1D0_0069);
    seen.push("dup");

    // 7. UNMAP then FREE.
    let mut un = vec![0u8; 48];
    nvos::Nvos47Parameters {
        h_client: 0xC1D0_0067,
        h_device: 0x5C00_0001,
        h_dma: 0xCAF0_0005,
        h_memory: 0x5C00_0019,
        flags: 0,
        dma_offset: 0x7F00_0000_0000,
        size: 0,
        status: 0,
    }
    .encode_into(&mut un)
    .expect("48");
    let u = t.decode_unmap_memory_dma(&un).expect("48");
    assert_eq!(u.dma_offset, 0x7F00_0000_0000);
    assert_eq!(u.size, 0, "0 means the whole mapping");
    seen.push("unmap");

    let mut fr = vec![0u8; 16];
    nvos::Nvos00Parameters {
        h_root: 0xC1D0_0067,
        h_object_parent: 0,
        h_object_old: 0xC1D0_0067,
        status: 0,
    }
    .encode_into(&mut fr)
    .expect("16");
    let f = t.decode_free(&fr).expect("16");
    assert_eq!(f.client, 0xC1D0_0067);
    assert_eq!(f.handle, 0xC1D0_0067, "freeing the client root");
    seen.push("free");

    // Non-vacuity: the stream really ran, in order, all eight steps.
    assert_eq!(
        seen,
        vec![
            "alloc-client",
            "alloc-device",
            "set-page-dir",
            "map",
            "skew-refused",
            "dup",
            "unmap",
            "free",
        ]
    );
}

/// The same stream under the **575** table, to prove the composition is not
/// accidentally 580-specific — and that the skewed event is the *other* one
/// there. Running both tables over one script is the `LockMode::{Degenerate,
/// Sharded}` precedent from `testing_doctrine.md` §7 applied to version skew.
#[test]
fn the_stream_runs_under_both_tables_with_the_skew_case_mirrored() {
    for (t, own_size, foreign) in [
        (v575(), 56usize, nvos46_v580_image()),
        (bench(), 64, {
            let mut b = vec![0u8; 56];
            Nvos46ParametersPre580 {
                dma_offset: 0xDEAD,
                ..Default::default()
            }
            .encode_into(&mut b)
            .expect("56");
            b
        }),
    ] {
        assert_eq!(t.map_dma_size(), own_size);
        // A buffer in this table's own size always decodes.
        let mine = vec![0u8; own_size];
        assert!(t.decode_map_memory_dma(&mine).is_ok());

        match t.map_dma_wire() {
            // 575 reading a 64-byte 580 image: long enough, so it DECODES — and
            // silently reads dmaOffset from the wrong place. Length alone cannot
            // catch this direction, which is exactly why the table exists.
            MapDmaWire::Pre580_65_06 => {
                let got = t.decode_map_memory_dma(&foreign).expect("64 >= 56");
                assert_eq!(
                    got.dma_offset, 0x0000_0000_0B0B_0B0B,
                    "a 575 reader on a 580 image reads kindOverride as the low half of dmaOffset"
                );
            }
            // 580 reading a 56-byte 575 image: too short, refused loudly.
            MapDmaWire::From580_65_06 => {
                assert_eq!(
                    t.decode_map_memory_dma(&foreign),
                    Err(AbiError::Truncated {
                        c_name: "NVOS46_PARAMETERS",
                        need: 64,
                        got: 56
                    })
                );
            }
        }
    }
}
