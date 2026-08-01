//! The split-VA-space page-directory publication (`0x20800a9f`), checked against a real
//! GA106 driver's own publication.
//!
//! ★★ The fixture is the C artifact's captured reply body for this control
//! (`C: src/qemu/mode2_initctrl_ga106.h`, `ctl_20800a9f`, rev `8baf4f2`). ⚠ Because the
//! struct is entirely `[in]`, that captured "reply" is the **request the real driver sent**,
//! echoed back by the real GSP — which makes it exactly the right input for a decoder: it is
//! a genuine publication produced by a stock 580.159.04 driver on real silicon, not one this
//! port composed.

use kayfabe_abi::gvaspacepdes::{
    COPY_SERVER_RESERVED_PDES_PARAMS_SIZE, GMMU_FMT_MAX_LEVELS, ServerReservedPdesError,
    decode_server_reserved_pdes, encode_server_reserved_pdes,
};

/// ⚠ The fixture is **176 bytes, not 184**: the C's recorder kept `dlen = 176` of a
/// `psize = 184` reply, exactly as it did for `0x20800a1f`. The eight missing bytes are the
/// tail of `levels[5]` — its `aperture` and `pageShift` and their padding — which a real
/// publication with `numLevelsToCopy = 4` leaves zero anyway.
///
/// ⊘ So the eight zeros below are **this port's**, not silicon's, and that is stated rather
/// than papered over: the round-trip test proves the decoder handles the whole struct, but
/// only the first 176 bytes of it are corroborated by real hardware. A recorder that had
/// kept the full `psize` would strengthen this test and nothing else here would change.
const ORACLE_CAPTURED_LEN: usize = 176;

fn oracle() -> Vec<u8> {
    let p = format!(
        "{}/tests/fixtures/ga106_ctl_20800a9f.bin",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut b = std::fs::read(&p).unwrap_or_else(|e| panic!("fixture {p} unreadable: {e}"));
    assert_eq!(
        b.len(),
        ORACLE_CAPTURED_LEN,
        "the fixture is the oracle's captured dlen; a different length means the extraction \
         changed and every assertion below is measuring something else"
    );
    b.resize(COPY_SERVER_RESERVED_PDES_PARAMS_SIZE, 0);
    b
}

/// ★★★ The whole licence for answering `NV_OK`, as one assertion: a **real** publication
/// decodes, and re-encodes to the identical 184 bytes.
///
/// The round trip is what makes the decoder load-bearing rather than decoration. An echo
/// would pass this trivially; a re-encode passes only if every field — including the two
/// padding gaps and the six `levels[]` entries past `numLevelsToCopy` — was accounted for.
#[test]
fn the_real_ga106_publication_decodes_and_round_trips_byte_for_byte() {
    let bytes = oracle();
    assert_eq!(bytes.len(), COPY_SERVER_RESERVED_PDES_PARAMS_SIZE);
    assert_eq!(bytes.len(), 184);
    assert!(
        bytes[ORACLE_CAPTURED_LEN..].iter().all(|b| *b == 0),
        "the padded tail must be the zeros this test supplied"
    );
    let p = decode_server_reserved_pdes(&bytes).expect("a real driver's publication decodes");
    assert_eq!(
        encode_server_reserved_pdes(&p),
        bytes,
        "the re-encode must be byte-identical, or a field was not understood"
    );
}

/// The values `ogkm-580` says this call carries during state load, read out of the real
/// capture. ⊘ Pinned because they are the evidence that the *decoder* agrees with the
/// *source reading* — `gpu_vaspace.c:5235` and `:5251-5256` and
/// `g_gpu_vaspace_nvoc.h:99-100` — and not merely with itself.
#[test]
fn the_publication_carries_the_split_vas_range_ogkm_names() {
    let p = decode_server_reserved_pdes(&oracle()).unwrap();
    assert_eq!(p.page_size, 1 << 21, "GMMU_PD0_VADDR_BIT_LO = 21");
    assert_eq!(
        p.virt_addr_lo, 0x1_0000_0000,
        "SPLIT_VAS_SERVER_RM_MANAGED_VA_START"
    );
    assert_eq!(
        p.virt_addr_hi, 0x1_1FFF_FFFF,
        "a 512 MiB span, inclusive of the last address"
    );
    assert_eq!(p.h_subdevice, 0, "unicast by subDeviceId");
    assert_eq!(p.subdevice_id, 0);
    assert_eq!(p.num_levels, 4, "GA106's client-RM PD chain is four deep");
    let shifts: Vec<u8> = p.levels[..4].iter().map(|l| l.page_shift).collect();
    assert_eq!(shifts, vec![47, 38, 29, 21]);
    for lv in &p.levels[..4] {
        assert_ne!(lv.size, 0);
        assert_ne!(lv.phys_address, 0);
    }
    for lv in &p.levels[4..] {
        assert_eq!(lv.size, 0, "unused levels are zero in a real publication");
    }
}

/// ⚠ The alignment rule is on `virtAddrHi + 1`, not on `virtAddrHi` — the header calls it
/// the **last** address (`ctrl90f1.h:293-296`). Reading it the natural way rejects every
/// legal publication, so this test pins the direction with the real value: `0x1_1FFF_FFFF`
/// is **not** 2 MiB-aligned and must still be accepted.
#[test]
fn the_upper_bound_is_the_last_address_not_the_end() {
    let p = decode_server_reserved_pdes(&oracle()).unwrap();
    assert_ne!(
        p.virt_addr_hi % p.page_size,
        0,
        "the last address is NOT aligned"
    );
    assert_eq!((p.virt_addr_hi + 1) % p.page_size, 0, "the end IS aligned");
}

fn mutated(f: impl Fn(&mut Vec<u8>)) -> Result<(), ServerReservedPdesError> {
    let mut b = oracle();
    f(&mut b);
    decode_server_reserved_pdes(&b).map(|_| ())
}

/// ⊘ Each of these is a payload a guest can send and this port refuses **by name**. Without
/// them the `NV_OK` would be a fall-through — an answer to any 184 bytes whatsoever — which
/// is what `#127`'s named-refusal default forbids.
#[test]
fn a_publication_that_contradicts_its_own_abi_is_refused() {
    // numLevelsToCopy past GMMU_FMT_MAX_LEVELS.
    assert_eq!(
        mutated(|b| b[0x20..0x24].copy_from_slice(&7u32.to_le_bytes())),
        Err(ServerReservedPdesError::LevelCountOutOfRange { got: 7 })
    );
    // …and zero levels is not a publication.
    assert_eq!(
        mutated(|b| b[0x20..0x24].copy_from_slice(&0u32.to_le_bytes())),
        Err(ServerReservedPdesError::LevelCountOutOfRange { got: 0 })
    );
    // A page size that is not a power of two makes both alignment rules unstatable.
    assert_eq!(
        mutated(|b| b[0x08..0x10].copy_from_slice(&0x30_0000u64.to_le_bytes())),
        Err(ServerReservedPdesError::PageSizeNotPowerOfTwo { got: 0x30_0000 })
    );
    assert_eq!(
        mutated(|b| b[0x08..0x10].copy_from_slice(&0u64.to_le_bytes())),
        Err(ServerReservedPdesError::PageSizeNotPowerOfTwo { got: 0 })
    );
    // virtAddrLo off its page.
    assert_eq!(
        mutated(|b| b[0x10..0x18].copy_from_slice(&0x1_0000_1000u64.to_le_bytes())),
        Err(ServerReservedPdesError::VirtAddrLoMisaligned {
            lo: 0x1_0000_1000,
            page_size: 1 << 21
        })
    );
    // virtAddrHi + 1 off its page.
    assert_eq!(
        mutated(|b| b[0x18..0x20].copy_from_slice(&0x1_1FFF_EFFFu64.to_le_bytes())),
        Err(ServerReservedPdesError::VirtAddrHiMisaligned {
            hi: 0x1_1FFF_EFFF,
            page_size: 1 << 21
        })
    );
    // An inverted range. Checked BEFORE alignment, so the reported fault is the real one.
    assert_eq!(
        mutated(|b| b[0x18..0x20].copy_from_slice(&0u64.to_le_bytes())),
        Err(ServerReservedPdesError::RangeInverted {
            lo: 0x1_0000_0000,
            hi: 0
        })
    );
    // A meaningful level of zero bytes.
    assert_eq!(
        mutated(|b| b[0x28 + 8..0x28 + 16].copy_from_slice(&0u64.to_le_bytes())),
        Err(ServerReservedPdesError::ZeroLevelSize { level: 0 })
    );
    // ⚠ …but a zero-sized level PAST numLevelsToCopy is normal and must be accepted: that is
    // what the real capture's tail is. A checker that scanned all six would refuse every
    // real publication.
    assert_eq!(
        mutated(|b| b[0x28 + 4 * 24 + 8..0x28 + 4 * 24 + 16].fill(0)),
        Ok(())
    );
    // Wrong length.
    assert_eq!(
        decode_server_reserved_pdes(&[0u8; 183]).unwrap_err(),
        ServerReservedPdesError::WrongSize { got: 183 }
    );
}

/// `virtAddrHi == u64::MAX` has no `+1`. Refused rather than wrapped — a wrap would make
/// `0` the end address and the alignment check would pass.
#[test]
fn an_unrepresentable_upper_bound_is_refused_rather_than_wrapped() {
    assert_eq!(
        mutated(|b| {
            b[0x10..0x18].copy_from_slice(&0u64.to_le_bytes());
            b[0x18..0x20].copy_from_slice(&u64::MAX.to_le_bytes());
        }),
        Err(ServerReservedPdesError::VirtAddrHiMisaligned {
            hi: u64::MAX,
            page_size: 1 << 21
        })
    );
}

/// The `levels[]` bound is `GMMU_FMT_MAX_LEVELS`, and the struct size is derived from it
/// rather than written down — so a change to one cannot silently disagree with the other.
#[test]
fn the_struct_size_is_derived_from_the_level_bound() {
    assert_eq!(GMMU_FMT_MAX_LEVELS, 6);
    assert_eq!(COPY_SERVER_RESERVED_PDES_PARAMS_SIZE, 0x28 + 6 * 24);
}
