//! Tests for [`crate::vbios`].
//!
//! # The oracle, and its honest limit
//!
//! [`parse`] below is a **transcription of the driver's own parser** — the
//! control flow of `kgspExtractVbiosFromRom_TU102`, `s_locateExpansionRoms`,
//! `s_vbiosFindBitHeader`, `s_vbiosParseFwsecUcodeDescFromBit` and
//! `s_vbiosFillFlcnUcodeFromDescV3`, in the same order, with the same
//! inequalities, citing the line it came from.
//!
//! ★ **A transcription is not an independent oracle.** If I misread the C, the
//! builder and this parser are wrong the *same* way and every test here still
//! passes. That is a real limit and it is why the deliverable is a guest boot,
//! not a green suite: only the actual `nvidia.ko` closes that gap.
//!
//! What the transcription *does* buy is the thing a boot cannot: a fast,
//! deterministic check that each individual structural field is load-bearing.
//! Every `rejects_*` test below poisons exactly one field of an image that is
//! otherwise known-good, and asserts the parser's verdict changes — so a field I
//! *think* is being read demonstrably is. Poison that produced no change would
//! mean the field is decorative (or that I put it in the wrong place), and the
//! test would fail rather than pass vacuously.

use super::*;
// Constants the *parser* needs but the builder does not: the 6-byte token form
// (the builder always emits the 8-byte one), the EXT code type (the builder only
// ever emits BASE), and the two FWSEC command codes — the BUILDER never names a
// command, it lays out the mapper and leaves `init_cmd` zero, so only the
// interface-walk transcription below has any business knowing what the driver
// writes there.
use crate::generated::vbios::{
    BIT_TOKEN_V1_00_SIZE_6, FALCON_APPLICATION_INTERFACE_DMEM_MAPPER_V3_CMD_FRTS,
    FALCON_APPLICATION_INTERFACE_DMEM_MAPPER_V3_CMD_SB, NV_BCRT_HASH_INFO_BASE_CODE_TYPE_VBIOS_EXT,
};

/// The subset of the driver's parse result the tests assert on.
#[derive(Debug, PartialEq, Eq)]
struct Parsed {
    bios_size: usize,
    expansion_rom_offset: usize,
    bit_addr: usize,
    desc_offset: usize,
    desc_size: usize,
    /// `signaturesTotalSize = descSize - 44` (`kernel_gsp_fwsec.c:985`).
    signatures_total_size: usize,
    /// `pUcode->size = RM_ALIGN_UP(StoredSize, 256)` (`:912`).
    ucode_size: u32,
    ucode_id: u8,
    vbios_version_combined: u64,
}

/// Every way the transcribed parser can refuse, named after the driver's own
/// `NV_PRINTF(LEVEL_ERROR, …)` or returned status.
#[derive(Debug, PartialEq, Eq)]
enum ParseFail {
    /// `kernel_gsp_vbios_tu102.c:507` — "did not find valid ROM signature".
    NoRomSignature,
    /// `:316-319` — `IS_VALID_PCI_DATA_SIG` failed.
    NoPcirSignature,
    /// `:519-524` — "expansion ROM has exceedingly large size".
    BiosTooLarge,
    /// `kernel_gsp_fwsec.c:1135` — "failed to find BIT header in VBIOS image".
    NoBitHeader,
    /// `:1144` — "failed to parse FWSEC ucode desc from VBIOS image".
    NoFwsecDesc,
    /// `:1152` — a bounds check in `s_vbiosFillFlcnUcodeFromDescV3` failed.
    BadUcodeOffsets,
    /// A read left the image.
    OutOfRange,
}

fn rd8(img: &[u8], at: usize) -> Result<u8, ParseFail> {
    img.get(at).copied().ok_or(ParseFail::OutOfRange)
}

fn rd16(img: &[u8], at: usize) -> Result<u16, ParseFail> {
    let s = img.get(at..at + 2).ok_or(ParseFail::OutOfRange)?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}

fn rd32(img: &[u8], at: usize) -> Result<u32, ParseFail> {
    let s = img.get(at..at + 4).ok_or(ParseFail::OutOfRange)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// A transcription of the driver's VBIOS → FWSEC parse.
///
/// `img` is what the PROM window serves; the driver reads it through
/// `NV_PROM_DATA`, which is why offsets here are image-relative.
fn parse(img: &[u8]) -> Result<Parsed, ParseFail> {
    // ── kgspExtractVbiosFromRom_TU102 (`:492-505`) ───────────────────────────
    // The signature is at offset 0, so `s_romImgFindPciHeader_TU102` (the
    // IFR/ROM-directory path) is never entered and `pciOffset` stays 0.
    let rom_sig = rd16(img, OFFSETOF_PCI_EXP_ROM_SIG)?;
    if rom_sig != PCI_EXP_ROM_SIGNATURE {
        return Err(ParseFail::NoRomSignature);
    }
    let pci_offset = 0usize;

    // ── s_locateExpansionRoms (`:275-416`) ───────────────────────────────────
    let mut curr_block = pci_offset;
    let mut ext_rom_offset = 0usize;
    let mut base_rom_size = 0usize;
    let (block_offset, block_size) = loop {
        let pci_blck = usize::from(rd16(
            img,
            curr_block + OFFSETOF_PCI_EXP_ROM_PCI_DATA_STRUCT_PTR,
        )?);
        let pcir = curr_block + pci_blck;

        if rd32(img, pcir + OFFSETOF_PCI_DATA_STRUCT_SIG)? != PCI_DATA_STRUCT_SIGNATURE {
            return Err(ParseFail::NoPcirSignature);
        }

        let mut is_last =
            (rd8(img, pcir + OFFSETOF_PCI_DATA_STRUCT_LAST_IMAGE)? & PCI_LAST_IMAGE) != 0;
        let img_len = rd16(img, pcir + OFFSETOF_PCI_DATA_STRUCT_IMAGE_LEN)?;
        let mut sub_img_len = img_len;

        // The PCI Data Extension, if present, OVERRIDES the image length (`:328-368`).
        let pcir_len = usize::from(rd16(img, pcir + OFFSETOF_PCI_DATA_STRUCT_LEN)?);
        let npde = (pcir + pcir_len + 0xF) & !0xF;
        if rd32(img, npde + OFFSETOF_PCI_DATA_EXT_STRUCT_SIG) == Ok(NV_PCI_DATA_EXT_SIG) {
            let rev = rd16(img, npde + OFFSETOF_PCI_DATA_EXT_STRUCT_REV)?;
            if rev == NV_PCI_DATA_EXT_REV_11 || rev == 0x100 {
                let npde_len = rd16(img, npde + OFFSETOF_PCI_DATA_EXT_STRUCT_LEN)?;
                sub_img_len = rd16(img, npde + OFFSETOF_PCI_DATA_EXT_STRUCT_SUBIMAGE_LEN)?;
                // The C is `OFFSETOF_… + sizeof(NvU8) <= nvPciDataExtLen`
                // (`kernel_gsp_vbios_tu102.c:355`); `+ 1 <=` is `<` on integers.
                if OFFSETOF_PCI_DATA_EXT_STRUCT_LAST_IMAGE < usize::from(npde_len) {
                    is_last = (rd8(img, npde + OFFSETOF_PCI_DATA_EXT_STRUCT_LAST_IMAGE)?
                        & PCI_LAST_IMAGE)
                        != 0;
                } else if sub_img_len < img_len {
                    is_last = false;
                }
            }
        }

        let code_type = rd8(img, pcir + OFFSETOF_PCI_DATA_STRUCT_CODE_TYPE)?;
        let block_offset = curr_block - pci_offset;
        let block_size = usize::from(sub_img_len) * PCI_ROM_IMAGE_BLOCK_SIZE;

        if ext_rom_offset == 0 && code_type == NV_BCRT_HASH_INFO_BASE_CODE_TYPE_VBIOS_EXT {
            ext_rom_offset = block_offset;
        } else if base_rom_size == 0 && code_type == NV_BCRT_HASH_INFO_BASE_CODE_TYPE_VBIOS_BASE {
            base_rom_size = block_size;
        }

        if is_last {
            break (block_offset, block_size);
        }
        curr_block += usize::from(sub_img_len) * PCI_ROM_IMAGE_BLOCK_SIZE;
        if curr_block >= img.len() {
            return Err(ParseFail::OutOfRange);
        }
    };

    let bios_size = block_offset + block_size;
    if bios_size > BIOS_MAX_SIZE {
        return Err(ParseFail::BiosTooLarge);
    }
    let expansion_rom_offset = if ext_rom_offset > 0 && base_rom_size > 0 {
        ext_rom_offset - base_rom_size
    } else {
        0
    };
    // The driver copies exactly `biosSize` bytes out of PROM; everything after
    // this point is bounded by it, not by the buffer we happen to hold.
    let img = img.get(..bios_size).ok_or(ParseFail::OutOfRange)?;

    // ── s_vbiosFindBitHeader (`kernel_gsp_fwsec.c:403-455`) ──────────────────
    let bit_addr = find_bit_header(img).ok_or(ParseFail::NoBitHeader)?;

    // ── s_vbiosParseFwsecUcodeDescFromBit (`:466-693`) ───────────────────────
    let header_size = usize::from(rd8(img, bit_addr + BIT_HEADER_SIZE_OFFSET)?);
    let token_size = usize::from(rd8(img, bit_addr + 9)?);
    let token_entries = usize::from(rd8(img, bit_addr + 10)?);
    if token_size < usize::from(BIT_TOKEN_V1_00_SIZE_6) {
        return Err(ParseFail::NoFwsecDesc);
    }
    let wide = token_size >= usize::from(BIT_TOKEN_V1_00_SIZE_8);

    let mut vbios_version_combined = 0u64;
    let mut found: Option<(usize, usize, u8)> = None;

    for tok in 0..token_entries {
        let t = bit_addr + header_size + tok * token_size;
        let Ok(token_id) = rd8(img, t) else { continue };
        let Ok(data_version) = rd8(img, t + 1) else {
            continue;
        };
        let Ok(data_size) = rd16(img, t + 2) else {
            continue;
        };
        let data_ptr = if wide {
            let Ok(v) = rd32(img, t + 4) else { continue };
            v as usize
        } else {
            let Ok(v) = rd16(img, t + 4) else { continue };
            usize::from(v)
        };

        // BIOSDATA — version capture only (`:534-552`).
        if token_id == BIT_TOKEN_BIOSDATA
            && data_version == BIT_DATA_BIOSDATA_VERSION_2
            && data_size > BIT_DATA_BIOSDATA_BINVER_SIZE_5
            && let (Ok(ver), Ok(oem)) = (rd32(img, data_ptr), rd8(img, data_ptr + 4))
        {
            vbios_version_combined = (u64::from(ver) << 8) | u64::from(oem);
        }

        // FALCON_DATA (`:555-560`).
        if token_id != BIT_TOKEN_FALCON_DATA
            || data_version != 2
            || data_size < BIT_DATA_FALCON_DATA_V2_SIZE_4
        {
            continue;
        }
        let Ok(table_ptr) = rd32(img, data_ptr) else {
            continue;
        };
        let table = expansion_rom_offset + table_ptr as usize;

        let (Ok(ver), Ok(hdr_size), Ok(entry_size), Ok(entry_count)) = (
            rd8(img, table),
            rd8(img, table + 1),
            rd8(img, table + 2),
            rd8(img, table + 3),
        ) else {
            continue;
        };
        if ver != FALCON_UCODE_TABLE_HDR_V1_VERSION
            || hdr_size < FALCON_UCODE_TABLE_HDR_V1_SIZE_6
            || entry_size < FALCON_UCODE_TABLE_ENTRY_V1_SIZE_6
        {
            continue;
        }

        for e in 0..usize::from(entry_count) {
            let ent = table + usize::from(hdr_size) + e * usize::from(entry_size);
            let (Ok(app_id), Ok(desc_ptr)) = (rd8(img, ent), rd32(img, ent + 2)) else {
                continue;
            };

            // ★ The skip condition, transcribed exactly (`:620-625`). Note that
            // appId 0x05 short-circuits the `&&`, so it matches under EITHER
            // debug-mode answer — which is why the builder emits it first.
            let use_debug = false;
            if app_id != FALCON_UCODE_ENTRY_APPID_FIRMWARE_SEC_LIC
                && ((use_debug && app_id != FALCON_UCODE_ENTRY_APPID_FWSEC_DBG)
                    || (!use_debug && app_id != FALCON_UCODE_ENTRY_APPID_FWSEC_PROD))
            {
                continue;
            }

            let desc_offset = expansion_rom_offset + desc_ptr as usize;
            let Ok(vdesc) = rd32(img, desc_offset) else {
                continue;
            };
            if NV_BIT_FALCON_UCODE_DESC_HEADER_VDESC_FLAGS_VERSION.get(vdesc) == 0 {
                continue; // "_UNAVAILABLE"
            }
            let desc_version = NV_BIT_FALCON_UCODE_DESC_HEADER_VDESC_VERSION.get(vdesc);
            let desc_size = NV_BIT_FALCON_UCODE_DESC_HEADER_VDESC_SIZE.get(vdesc) as usize;
            if desc_version != NV_BIT_FALCON_UCODE_DESC_HEADER_VDESC_VERSION_V3
                || desc_size < FALCON_UCODE_DESC_V3_SIZE_44
            {
                continue;
            }
            found = Some((desc_offset, desc_size, app_id));
            break;
        }
        if found.is_some() {
            break;
        }
    }
    let (desc_offset, desc_size, _app) = found.ok_or(ParseFail::NoFwsecDesc)?;

    // ── s_vbiosFillFlcnUcodeFromDescV3 (`:877-1021`) ─────────────────────────
    let stored_size = rd32(img, desc_offset + 4)?;
    let pkc_data_offset = rd32(img, desc_offset + 8)?;
    let imem_load_size = rd32(img, desc_offset + 20)?;
    let dmem_load_size = rd32(img, desc_offset + 32)?;
    let ucode_id = rd8(img, desc_offset + 38)?;

    let size = stored_size.div_ceil(UCODE_ALIGN) * UCODE_ALIGN;
    let image_offset = desc_offset
        .checked_add(desc_size)
        .ok_or(ParseFail::BadUcodeOffsets)?;
    if image_offset >= bios_size {
        return Err(ParseFail::BadUcodeOffsets);
    }
    if image_offset + size as usize > bios_size {
        return Err(ParseFail::BadUcodeOffsets);
    }
    if imem_load_size > size {
        return Err(ParseFail::BadUcodeOffsets);
    }
    let data_offset = imem_load_size;
    if data_offset >= size {
        return Err(ParseFail::BadUcodeOffsets);
    }
    if data_offset + dmem_load_size > size {
        return Err(ParseFail::BadUcodeOffsets);
    }
    let sig_data_offset = data_offset + pkc_data_offset;
    if sig_data_offset >= size {
        return Err(ParseFail::BadUcodeOffsets);
    }
    if sig_data_offset + BCRT30_RSA3K_SIG_SIZE as u32 > size {
        return Err(ParseFail::BadUcodeOffsets);
    }
    if desc_size < FALCON_UCODE_DESC_V3_SIZE_44 {
        return Err(ParseFail::BadUcodeOffsets);
    }

    Ok(Parsed {
        bios_size,
        expansion_rom_offset,
        bit_addr,
        desc_offset,
        desc_size,
        signatures_total_size: desc_size - FALCON_UCODE_DESC_V3_SIZE_44,
        ucode_size: size,
        ucode_id,
        vbios_version_combined,
    })
}

/// `s_vbiosFindBitHeader`, transcribed (`kernel_gsp_fwsec.c:403-455`): scan for
/// the id/signature pair, then require the header's bytes to sum to 0 mod 256.
fn find_bit_header(img: &[u8]) -> Option<usize> {
    for addr in 0..img.len().saturating_sub(3) {
        if rd16(img, addr) != Ok(BIT_HEADER_ID) || rd32(img, addr + 2) != Ok(BIT_HEADER_SIGNATURE) {
            continue;
        }
        let header_size = usize::from(rd8(img, addr + BIT_HEADER_SIZE_OFFSET).ok()?);
        let mut sum = 0u32;
        for j in 0..header_size {
            sum += u32::from(rd8(img, addr + j).ok()?);
        }
        if sum & 0xFF == 0 {
            return Some(addr);
        }
    }
    None
}

fn ga106() -> &'static VbiosProfile {
    profile_for_device_id(0x2504).expect("the GA106 row is in VBIOS_PROFILES")
}

fn good() -> Vec<u8> {
    build(ga106(), VbiosWire::Tu102Bit).expect("the shipped profile must build")
}

/// The DMEM structures the builder actually **writes into the image**, as
/// `(name, payload-relative offset, length)`.
///
/// ★ Deliberately NOT the same list `build` bounds-checks. The command buffers
/// and `hsSigDmemAddr` are *destinations the driver fills at run time* — in the
/// ROM image those ranges are still marker fill, and treating them as written
/// would blind the marker check over 896 bytes for no reason.
fn dmem_structures(p: &VbiosProfile) -> [(&'static str, usize, usize); 2] {
    let fw = &p.fwsec;
    let base = fw.imem_load_size as usize;
    [
        (
            "the interface header and its entries",
            base + fw.interface_offset as usize,
            FalconApplicationInterfaceHeaderV1::SIZE
                + usize::from(INTERFACE_ENTRY_COUNT) * FalconApplicationInterfaceEntryV1::SIZE,
        ),
        (
            "the DMEM mapper",
            base + fw.dmem_mapper_offset as usize,
            FalconApplicationInterfaceDmemMapperV3::SIZE,
        ),
    ]
}

/// The payload bytes of a built image, i.e. what the driver hands the falcon.
fn payload_of(img: &[u8]) -> &[u8] {
    let p = parse(img).expect("a generated image must parse");
    let at = p.desc_offset + p.desc_size;
    &img[at..at + p.ucode_size as usize]
}

/// The DMEM section of a built image — `pMappedData` as
/// `s_prepareForFwsec_TU102` computes it (`ogkm-580:
/// src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_frts_tu102.c:394-396`).
fn dmem_of(img: &[u8], p: &VbiosProfile) -> Vec<u8> {
    let pay = payload_of(img);
    pay[p.fwsec.imem_load_size as usize..][..p.fwsec.dmem_load_size as usize].to_vec()
}

// ─── Acceptance ──────────────────────────────────────────────────────────────

/// The end-to-end property: the shipped profile produces an image the
/// transcribed driver parser accepts, and the values it recovers are the ones
/// the profile declared.
#[test]
fn the_shipped_profile_builds_an_image_the_driver_parser_accepts() {
    let img = good();
    let p = parse(&img).expect("a generated image must parse");

    let fw = &ga106().fwsec;
    assert_eq!(p.expansion_rom_offset, 0, "a single BASE image yields 0");
    assert_eq!(p.bios_size, img.len(), "biosSize covers the whole image");
    assert_eq!(
        p.bios_size % PCI_ROM_IMAGE_BLOCK_SIZE,
        0,
        "the image is a whole number of 512-byte blocks"
    );
    assert_eq!(
        p.desc_size,
        FALCON_UCODE_DESC_V3_SIZE_44 + usize::from(fw.signature_count) * BCRT30_RSA3K_SIG_SIZE
    );
    assert_eq!(
        p.signatures_total_size,
        usize::from(fw.signature_count) * BCRT30_RSA3K_SIG_SIZE,
        "signaturesTotalSize = descSize - 44 must equal the blob we wrote"
    );
    assert_eq!(p.ucode_size, fw.imem_load_size + fw.dmem_load_size);
    assert_eq!(p.ucode_id, fw.ucode_id);
    assert_eq!(
        p.vbios_version_combined,
        (u64::from(ga106().vbios_version) << 8) | u64::from(ga106().vbios_oem_version),
        "the BIOSDATA token round-trips the declared version"
    );
    assert!(p.bios_size <= BIOS_MAX_SIZE);
}

/// The `0xAA55` at offset 0 is what the driver looked for and did not find. Its
/// absence is the exact failure this whole module exists to remove, so assert
/// the bytes directly and not only through the parser.
#[test]
fn the_rom_signature_is_the_first_two_bytes() {
    let img = good();
    assert_eq!(
        u16::from_le_bytes([img[0], img[1]]),
        PCI_EXP_ROM_SIGNATURE,
        "0xAA55 at offset 0 — the byte pattern whose absence produced \
         `did not find valid ROM signature`"
    );
}

/// ★ The signature blob and the ucode payload must not accidentally contain a
/// second BIT-header candidate: `s_vbiosFindBitHeader` returns the FIRST one
/// whose checksum passes, so a colliding pattern earlier in the image would
/// silently redirect the whole parse.
#[test]
fn exactly_one_bit_header_candidate() {
    let img = good();
    let mut hits = Vec::new();
    for addr in 0..img.len().saturating_sub(6) {
        if rd16(&img, addr) == Ok(BIT_HEADER_ID) && rd32(&img, addr + 2) == Ok(BIT_HEADER_SIGNATURE)
        {
            hits.push(addr);
        }
    }
    assert_eq!(hits.len(), 1, "expected one BIT header, found {hits:?}");
    assert_eq!(hits[0], parse(&img).unwrap().bit_addr);
}

/// A single BASE-code-type image is what makes `expansionRomOffset` zero. Assert
/// the declared code type is BASE and not EXT, because the parser's
/// `extRomOffset - baseRomSize` arithmetic is what would otherwise shift every
/// `expansionRomOffset + ptr` in the FWSEC parse.
#[test]
fn the_single_image_declares_base_not_ext() {
    let img = good();
    let pcir = usize::from(rd16(&img, OFFSETOF_PCI_EXP_ROM_PCI_DATA_STRUCT_PTR).unwrap());
    let code = rd8(&img, pcir + OFFSETOF_PCI_DATA_STRUCT_CODE_TYPE).unwrap();
    assert_eq!(code, NV_BCRT_HASH_INFO_BASE_CODE_TYPE_VBIOS_BASE);
    assert_ne!(
        code, NV_BCRT_HASH_INFO_BASE_CODE_TYPE_VBIOS_EXT,
        "an EXT-only image would give extRomOffset>0, baseRomSize==0, and still \
         expansionRomOffset==0 — but for the wrong reason"
    );
}

/// FWSEC is reachable under **either** answer to `kgspIsDebugModeEnabled_HAL`,
/// because the first entry uses the appId that short-circuits the debug test.
#[test]
fn fwsec_is_found_under_both_debug_and_prod() {
    let img = good();
    let p = parse(&img).unwrap();
    // Re-run the entry scan for both settings of the flag and require the same
    // descriptor both times.
    for use_debug in [false, true] {
        let found = scan_entries(&img, use_debug).expect("an entry must match");
        assert_eq!(
            found, p.desc_offset,
            "debug={use_debug} must reach the same descriptor"
        );
    }
}

/// The entry-selection half of `s_vbiosParseFwsecUcodeDescFromBit`, parameterised
/// on the debug flag the real driver reads from a fuse.
fn scan_entries(img: &[u8], use_debug: bool) -> Option<usize> {
    let bit = find_bit_header(img)?;
    let header_size = usize::from(rd8(img, bit + BIT_HEADER_SIZE_OFFSET).ok()?);
    let token_size = usize::from(rd8(img, bit + 9).ok()?);
    let entries = usize::from(rd8(img, bit + 10).ok()?);
    for tok in 0..entries {
        let t = bit + header_size + tok * token_size;
        if rd8(img, t).ok()? != BIT_TOKEN_FALCON_DATA {
            continue;
        }
        let table = rd32(img, rd32(img, t + 4).ok()? as usize).ok()? as usize;
        let hdr_size = usize::from(rd8(img, table + 1).ok()?);
        let entry_size = usize::from(rd8(img, table + 2).ok()?);
        let count = usize::from(rd8(img, table + 3).ok()?);
        for e in 0..count {
            let ent = table + hdr_size + e * entry_size;
            let app = rd8(img, ent).ok()?;
            if app != FALCON_UCODE_ENTRY_APPID_FIRMWARE_SEC_LIC
                && ((use_debug && app != FALCON_UCODE_ENTRY_APPID_FWSEC_DBG)
                    || (!use_debug && app != FALCON_UCODE_ENTRY_APPID_FWSEC_PROD))
            {
                continue;
            }
            return Some(rd32(img, ent + 2).ok()? as usize);
        }
    }
    None
}

// ─── Non-vacuity: poison one field, watch the parser change its verdict ──────
//
// Each test asserts the GOOD image parses first, so a test that "passes" because
// the image was already broken is impossible.

/// Poison `at` with `v`, having first proved the pristine image parses and that
/// the byte actually changes.
fn poisoned(at: usize, v: u8) -> Vec<u8> {
    let mut img = good();
    assert!(parse(&img).is_ok(), "the pristine image must parse first");
    assert_ne!(
        img[at], v,
        "the poison must actually change byte {at:#x} — otherwise the test is vacuous"
    );
    img[at] = v;
    img
}

#[test]
fn rejects_a_corrupt_rom_signature() {
    let img = poisoned(0, 0x00);
    assert_eq!(parse(&img), Err(ParseFail::NoRomSignature));
}

#[test]
fn rejects_a_corrupt_pcir_signature() {
    let good = good();
    let pcir = usize::from(rd16(&good, OFFSETOF_PCI_EXP_ROM_PCI_DATA_STRUCT_PTR).unwrap());
    let img = poisoned(pcir + OFFSETOF_PCI_DATA_STRUCT_SIG, 0x00);
    assert_eq!(parse(&img), Err(ParseFail::NoPcirSignature));
}

/// Clearing the last-image bit makes `s_locateExpansionRoms` walk to the next
/// block, which does not exist — the `for(;;)` has no bound of its own.
#[test]
fn rejects_a_cleared_last_image_bit() {
    let good = good();
    let pcir = usize::from(rd16(&good, OFFSETOF_PCI_EXP_ROM_PCI_DATA_STRUCT_PTR).unwrap());
    let npde =
        (pcir + usize::from(rd16(&good, pcir + OFFSETOF_PCI_DATA_STRUCT_LEN).unwrap()) + 0xF)
            & !0xF;
    // The NPDE's last-image byte OVERRIDES the PCIR's, so that is the one to clear.
    let img = poisoned(npde + OFFSETOF_PCI_DATA_EXT_STRUCT_LAST_IMAGE, 0x00);
    assert_eq!(parse(&img), Err(ParseFail::OutOfRange));
}

/// Breaking the BIT header's `Id` removes the only candidate, so the scan runs
/// to the end and reports not-found.
#[test]
fn rejects_a_corrupt_bit_header_id() {
    let bit = parse(&good()).unwrap().bit_addr;
    let img = poisoned(bit, 0x00);
    assert_eq!(parse(&img), Err(ParseFail::NoBitHeader));
}

/// ★ The one-byte checksum is real and is enforced. This is the sharpest of the
/// structural tests: nothing about the image's *shape* changes, only one byte
/// that exists solely to make a sum come out to zero.
#[test]
fn rejects_a_broken_bit_header_checksum() {
    let bit = parse(&good()).unwrap().bit_addr;
    let img = poisoned(bit + 11, 0xFF);
    assert_eq!(
        parse(&img),
        Err(ParseFail::NoBitHeader),
        "a header whose bytes do not sum to 0 mod 256 is not a BIT header"
    );
}

/// The FALCON_DATA token id is what leads to FWSEC at all.
#[test]
fn rejects_a_corrupt_falcon_data_token_id() {
    let good = good();
    let bit = parse(&good).unwrap().bit_addr;
    let header_size = usize::from(good[bit + BIT_HEADER_SIZE_OFFSET]);
    let token_size = usize::from(good[bit + 9]);
    // Token 1 is FALCON_DATA (token 0 is BIOSDATA).
    let mut img = poisoned(bit + header_size + token_size, 0x00);
    // Fix the header checksum so the failure is attributable to the token, not
    // to the header — otherwise this test would prove the previous one again.
    fix_bit_checksum(&mut img, bit);
    assert_eq!(parse(&img), Err(ParseFail::NoFwsecDesc));
}

/// A ucode table header claiming the wrong version is skipped (`:587-592`).
#[test]
fn rejects_a_wrong_ucode_table_version() {
    let good = good();
    let bit = parse(&good).unwrap().bit_addr;
    let header_size = usize::from(good[bit + BIT_HEADER_SIZE_OFFSET]);
    let token_size = usize::from(good[bit + 9]);
    let t = bit + header_size + token_size;
    let data_ptr = rd32(&good, t + 4).unwrap() as usize;
    let table = rd32(&good, data_ptr).unwrap() as usize;
    let img = poisoned(table, 0x02); // Version != 1
    assert_eq!(parse(&img), Err(ParseFail::NoFwsecDesc));
}

/// ★ All three appIds must be poisoned together, which is itself the proof that
/// the builder's three-entry redundancy works: breaking any one leaves FWSEC
/// reachable.
#[test]
fn all_three_appids_must_be_broken_before_fwsec_is_lost() {
    let good = good();
    let bit = parse(&good).unwrap().bit_addr;
    let header_size = usize::from(good[bit + BIT_HEADER_SIZE_OFFSET]);
    let token_size = usize::from(good[bit + 9]);
    let t = bit + header_size + token_size;
    let table = rd32(&good, rd32(&good, t + 4).unwrap() as usize).unwrap() as usize;
    let hdr_size = usize::from(good[table + 1]);
    let entry_size = usize::from(good[table + 2]);
    let count = usize::from(good[table + 3]);
    assert_eq!(count, 3, "the builder emits three FWSEC entries");

    // Breaking only the first still parses — the prod entry is still there.
    let mut one = good.clone();
    one[table + hdr_size] = 0x00;
    assert!(
        parse(&one).is_ok(),
        "breaking one appId must NOT lose FWSEC — that is the point of three"
    );

    // Breaking all three loses it.
    let mut all = good.clone();
    for e in 0..count {
        all[table + hdr_size + e * entry_size] = 0x00;
    }
    assert_eq!(parse(&all), Err(ParseFail::NoFwsecDesc));
}

/// Clearing the `vDesc` "version available" flag makes the entry skipped with
/// `unexpected ucode desc version missing`.
#[test]
fn rejects_a_descriptor_with_no_version_flag() {
    let good = good();
    let desc = parse(&good).unwrap().desc_offset;
    let vdesc = rd32(&good, desc).unwrap();
    let cleared = vdesc & !NV_BIT_FALCON_UCODE_DESC_HEADER_VDESC_FLAGS_VERSION.mask();
    assert_ne!(vdesc, cleared, "the flag must have been set to begin with");
    let mut img = good.clone();
    img[desc..desc + 4].copy_from_slice(&cleared.to_le_bytes());
    assert_eq!(parse(&img), Err(ParseFail::NoFwsecDesc));
}

/// A descriptor claiming a size smaller than 44 is refused — and this is the
/// field that also determines `signaturesTotalSize`.
#[test]
fn rejects_a_descriptor_size_below_the_v3_minimum() {
    let good = good();
    let desc = parse(&good).unwrap().desc_offset;
    let vdesc = rd32(&good, desc).unwrap();
    let shrunk = (vdesc & !NV_BIT_FALCON_UCODE_DESC_HEADER_VDESC_SIZE.mask())
        | NV_BIT_FALCON_UCODE_DESC_HEADER_VDESC_SIZE.set(43);
    let mut img = good.clone();
    img[desc..desc + 4].copy_from_slice(&shrunk.to_le_bytes());
    assert_eq!(parse(&img), Err(ParseFail::NoFwsecDesc));
}

/// An `IMEMLoadSize` larger than the whole ucode fails `imemSize > size`.
#[test]
fn rejects_an_imem_size_larger_than_the_ucode() {
    let good = good();
    let desc = parse(&good).unwrap().desc_offset;
    let mut img = good.clone();
    img[desc + 20..desc + 24].copy_from_slice(&0x00FF_0000u32.to_le_bytes());
    assert_eq!(parse(&img), Err(ParseFail::BadUcodeOffsets));
}

/// A `PKCDataOffset` that pushes the 384-byte signature past the end of the
/// ucode fails `sigDataOffset + sigSize > size`.
#[test]
fn rejects_a_pkc_offset_that_overruns_the_signature() {
    let good = good();
    let desc = parse(&good).unwrap().desc_offset;
    let mut img = good.clone();
    // dataOffset is imem (0x1000) and size is 0x2000, so 0x1000 - 383 overruns.
    img[desc + 8..desc + 12].copy_from_slice(&(0x1000u32 - 383).to_le_bytes());
    assert_eq!(parse(&img), Err(ParseFail::BadUcodeOffsets));
}

/// Shrinking the declared image length truncates `biosSize` so the ucode payload
/// no longer fits — `imageOffset + size > biosSize`.
#[test]
fn rejects_an_image_length_that_truncates_the_ucode() {
    let good = good();
    let pcir = usize::from(rd16(&good, OFFSETOF_PCI_EXP_ROM_PCI_DATA_STRUCT_PTR).unwrap());
    let npde =
        (pcir + usize::from(rd16(&good, pcir + OFFSETOF_PCI_DATA_STRUCT_LEN).unwrap()) + 0xF)
            & !0xF;
    let mut img = good.clone();
    // The NPDE sub-image length is the one that actually advances/bounds the walk.
    img[npde + OFFSETOF_PCI_DATA_EXT_STRUCT_SUBIMAGE_LEN
        ..npde + OFFSETOF_PCI_DATA_EXT_STRUCT_SUBIMAGE_LEN + 2]
        .copy_from_slice(&1u16.to_le_bytes());
    assert!(
        matches!(
            parse(&img),
            Err(ParseFail::BadUcodeOffsets | ParseFail::NoBitHeader | ParseFail::OutOfRange)
        ),
        "a 512-byte biosSize cannot contain the descriptor or its payload"
    );
}

fn fix_bit_checksum(img: &mut [u8], bit: usize) {
    let header_size = usize::from(img[bit + BIT_HEADER_SIZE_OFFSET]);
    img[bit + 11] = 0;
    let sum = img[bit..bit + header_size]
        .iter()
        .fold(0u8, |a, b| a.wrapping_add(*b));
    img[bit + 11] = 0u8.wrapping_sub(sum);
}

// ─── The builder's own refusals ──────────────────────────────────────────────

#[test]
fn refuses_a_profile_with_no_signatures() {
    let mut p = *ga106();
    p.fwsec.signature_count = 0;
    assert_eq!(
        build(&p, VbiosWire::Tu102Bit),
        Err(VbiosError::NoSignatures)
    );
}

/// `44 + n*384` must fit a 16-bit field; 171 signatures is the first count that
/// does not.
#[test]
fn refuses_a_descriptor_too_large_for_the_16_bit_size_field() {
    let mut p = *ga106();
    p.fwsec.signature_count = 171;
    assert!(matches!(
        build(&p, VbiosWire::Tu102Bit),
        Err(VbiosError::DescriptorTooLarge { .. })
    ));
    // …and 170 still fits, so the boundary is where it is claimed to be.
    p.fwsec.signature_count = 170;
    assert!(build(&p, VbiosWire::Tu102Bit).is_ok());
}

#[test]
fn refuses_a_signature_outside_the_ucode() {
    let mut p = *ga106();
    p.fwsec.pkc_data_offset = p.fwsec.dmem_load_size; // leaves no room for 384 bytes
    assert!(matches!(
        build(&p, VbiosWire::Tu102Bit),
        Err(VbiosError::SignatureOutOfUcode { .. })
    ));
}

#[test]
fn refuses_an_image_over_the_1mb_cap() {
    let mut p = *ga106();
    p.fwsec.imem_load_size = 0x0010_0000;
    assert!(matches!(
        build(&p, VbiosWire::Tu102Bit),
        Err(VbiosError::ImageTooLarge { .. })
    ));
}

#[test]
fn an_unknown_device_id_is_a_fault_not_a_fallback() {
    assert_eq!(
        profile_for_device_id(0xDEAD),
        Err(VbiosError::NoProfileForDevice { device_id: 0xDEAD })
    );
}

/// Every shipped profile must build and parse — a row added to the table cannot
/// silently be one that does not work.
#[test]
fn every_shipped_profile_builds_and_parses() {
    assert!(!VBIOS_PROFILES.is_empty(), "the table must not be empty");
    for p in VBIOS_PROFILES {
        let img = build(p, VbiosWire::Tu102Bit)
            .unwrap_or_else(|e| panic!("profile {} failed to build: {e}", p.name));
        let parsed =
            parse(&img).unwrap_or_else(|e| panic!("profile {} failed to parse: {e:?}", p.name));
        assert_eq!(parsed.ucode_id, p.fwsec.ucode_id, "profile {}", p.name);
    }
}

/// The build is deterministic — the image is a pure function of the profile.
#[test]
fn the_build_is_deterministic() {
    assert_eq!(good(), good());
}

/// ★ The ucode payload begins **immediately** after the descriptor and its
/// signatures, with no alignment gap, because `imageOffset = descOffset +
/// descSize` (`kernel_gsp_fwsec.c:937`) is where the driver reads it from.
///
/// This test exists because of a measured hole: while the payload was
/// zero-filled it was byte-identical to the surrounding padding, and shifting
/// its write offset by 16 bytes left **all 25 other tests green**. The marker
/// pattern plus this assertion is what closes that.
#[test]
fn the_ucode_payload_sits_immediately_after_the_signatures() {
    let img = good();
    let p = parse(&img).unwrap();
    let image_offset = p.desc_offset + p.desc_size;

    // ★ The DMEM structures are laid OVER the marker fill, so they are the only
    // payload bytes that are legitimately not marker bytes. Skipping them costs
    // this test some reach, and the `checked` count below is what stops the skip
    // from quietly growing to swallow the assertion — a hole punched by a later
    // profile change makes the count wrong rather than making the test vacuous.
    let structures = dmem_structures(ga106());
    let skipped: usize = structures.iter().map(|&(_, _, len)| len).sum();
    let mut checked = 0usize;
    for i in 0..p.ucode_size as usize {
        if structures
            .iter()
            .any(|&(_, at, len)| (at..at + len).contains(&i))
        {
            continue;
        }
        assert_eq!(
            img[image_offset + i],
            UCODE_MARKER ^ (i % 251) as u8,
            "ucode payload byte {i} is not at descOffset+descSize ({image_offset:#x})"
        );
        checked += 1;
    }
    assert_eq!(
        checked,
        p.ucode_size as usize - skipped,
        "the marker check covered the payload minus exactly the DMEM structures"
    );
    // …and the byte just before it is the last signature byte, not padding — so
    // there is provably no gap.
    let last_sig = p.signatures_total_size - 1;
    assert_eq!(
        img[image_offset - 1],
        (last_sig % 251) as u8,
        "the signature blob must run right up to the payload"
    );
}

// ─── The FWSEC application interface table ───────────────────────────────────
//
// The plane the ROM parser above cannot see. `kgspParseFwsecUcodeFromVbiosImg`
// reads `InterfaceOffset` as an opaque number and copies the payload; it is
// `s_prepareForFwsec_TU102` → `s_vbiosPatchInterfaceData`, one whole stage later
// and in a different file, that walks what the number points at. A guest whose
// ROM parses perfectly still stops there with
// `failed to find required interface entry for FWSEC cmd 0x15`.
//
// ★ Same honest limit as `parse` above, and it bites harder here: this is a
// transcription of a function the compiled oracle did not run either until this
// commit extended it. The independent check is
// `tests/tests/vbios_real_parser_oracle.rs`, which compiles the real
// `s_vbiosPatchInterfaceData` and runs it over this image; above that, a boot.

/// Every way the driver's interface walk refuses, named after its own diagnostic
/// or returned status
/// (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_frts_tu102.c:163-262`).
#[derive(Debug, PartialEq, Eq)]
enum PatchFail {
    /// `:180-186,199-208,215-224,406-413` — any `NV_ERR_INVALID_OFFSET`.
    InvalidOffset,
    /// `:190-194` — "too few interface entires found for FWSEC cmd 0x%x".
    TooFewEntries,
    /// `:230-234` — "failed to find required interface entry for FWSEC cmd 0x%x".
    /// **This is the message the stock guest stopped on.**
    NoDmemMapper,
}

/// What a successful walk produces.
#[derive(Debug, PartialEq, Eq)]
struct Patched {
    /// The DMEM section after the patch — `init_cmd` written, command copied in.
    dmem: Vec<u8>,
    /// Where the mapper was found.
    mapper_at: u32,
    /// Whether the driver emitted "insufficient cmd buffer" (a warning only: it
    /// copies regardless, `:238-241`).
    warned_small_buffer: bool,
}

/// A transcription of `s_vbiosPatchInterfaceData`, including its quirks.
///
/// ★ Two of those quirks are load-bearing and are reproduced rather than tidied:
/// the walk keeps the **last** `_DMEMMAPPER` it sees, and the final bounds check
/// tests `pIntFaceEntry->dmemOffset + cmdBufferSize` — the *entry's* offset —
/// where the copy it guards uses `pDmemMapper->cmd_in_buffer_offset` (`:243-254`).
/// A tidied transcription would accept tables the driver rejects and vice versa.
fn patch_interface_data(
    dmem: &[u8],
    cmd: u32,
    cmd_buffer: &[u8],
    interface_offset: u32,
) -> Result<Patched, PatchFail> {
    let mapped = dmem.len() as u32;
    let hdr_size = FalconApplicationInterfaceHeaderV1::SIZE as u32;
    let entry_size = FalconApplicationInterfaceEntryV1::SIZE as u32;
    let mapper_size = FalconApplicationInterfaceDmemMapperV3::SIZE as u32;
    let cmd_len = cmd_buffer.len() as u32;

    if interface_offset >= mapped {
        return Err(PatchFail::InvalidOffset);
    }
    let mut next = interface_offset
        .checked_add(hdr_size)
        .ok_or(PatchFail::InvalidOffset)?;
    if next > mapped {
        return Err(PatchFail::InvalidOffset);
    }
    let hdr = FalconApplicationInterfaceHeaderV1::decode(&dmem[interface_offset as usize..])
        .map_err(|_| PatchFail::InvalidOffset)?;
    if hdr.entry_count < 2 {
        return Err(PatchFail::TooFewEntries);
    }

    let mut cur = next;
    let mut mapper_at: Option<u32> = None;
    let mut last_entry: Option<FalconApplicationInterfaceEntryV1> = None;
    for _ in 0..hdr.entry_count {
        if cur >= mapped {
            return Err(PatchFail::InvalidOffset);
        }
        next = cur
            .checked_add(entry_size)
            .ok_or(PatchFail::InvalidOffset)?;
        if next > mapped {
            return Err(PatchFail::InvalidOffset);
        }
        let entry = FalconApplicationInterfaceEntryV1::decode(&dmem[cur as usize..])
            .map_err(|_| PatchFail::InvalidOffset)?;
        cur = next;
        if entry.id == FALCON_APPLICATION_INTERFACE_ENTRY_ID_DMEMMAPPER {
            if entry.dmem_offset >= mapped {
                return Err(PatchFail::InvalidOffset);
            }
            let end = entry
                .dmem_offset
                .checked_add(mapper_size)
                .ok_or(PatchFail::InvalidOffset)?;
            if end > mapped {
                return Err(PatchFail::InvalidOffset);
            }
            mapper_at = Some(entry.dmem_offset);
        }
        last_entry = Some(entry);
    }
    let mapper_at = mapper_at.ok_or(PatchFail::NoDmemMapper)?;

    let mut out = dmem.to_vec();
    let mut mapper = FalconApplicationInterfaceDmemMapperV3::decode(&out[mapper_at as usize..])
        .map_err(|_| PatchFail::InvalidOffset)?;
    mapper.init_cmd = cmd;
    let warned_small_buffer = mapper.cmd_in_buffer_size < cmd_len;
    if mapper.cmd_in_buffer_offset >= mapped {
        return Err(PatchFail::InvalidOffset);
    }
    // The quirk: the *entry's* offset, not the buffer's.
    let quirk = last_entry
        .map(|e| e.dmem_offset)
        .ok_or(PatchFail::InvalidOffset)?;
    if quirk.checked_add(cmd_len).ok_or(PatchFail::InvalidOffset)? > mapped {
        return Err(PatchFail::InvalidOffset);
    }
    let _ = mapper.encode_into(&mut out[mapper_at as usize..]);
    let at = mapper.cmd_in_buffer_offset as usize;
    out[at..at + cmd_buffer.len()].copy_from_slice(cmd_buffer);
    Ok(Patched {
        dmem: out,
        mapper_at,
        warned_small_buffer,
    })
}

/// A stand-in for whatever `s_prepareForFwsec_TU102` builds — the walk copies it
/// verbatim and never inspects it, so its *content* is irrelevant and its
/// *length* is the only thing that matters. [`FWSEC_CMD_BUFFER_MIN`] is the
/// larger of the two real payloads' floor.
fn fake_cmd_buffer() -> Vec<u8> {
    (0..FWSEC_CMD_BUFFER_MIN).map(|i| (i % 251) as u8).collect()
}

/// ★ THE ROW THIS COMMIT ADDS: the DMEM payload now describes itself well enough
/// for the driver to deliver a FWSEC command into it.
#[test]
fn the_dmem_payload_carries_an_interface_table_the_driver_can_walk() {
    let img = good();
    let p = ga106();
    let dmem = dmem_of(&img, p);
    let cmd = fake_cmd_buffer();

    let out = patch_interface_data(
        &dmem,
        FALCON_APPLICATION_INTERFACE_DMEM_MAPPER_V3_CMD_FRTS,
        &cmd,
        p.fwsec.interface_offset,
    )
    .expect("the walk must find a DMEM mapper");

    assert_eq!(out.mapper_at, p.fwsec.dmem_mapper_offset);
    assert!(
        !out.warned_small_buffer,
        "cmd_in_buffer_size must hold the command the driver builds"
    );

    // `init_cmd` was 0 in the image and is 0x15 after the patch — which is the
    // whole reason the builder emits 0 there.
    let before =
        FalconApplicationInterfaceDmemMapperV3::decode(&dmem[out.mapper_at as usize..]).unwrap();
    let after = FalconApplicationInterfaceDmemMapperV3::decode(&out.dmem[out.mapper_at as usize..])
        .unwrap();
    assert_eq!(
        before.init_cmd, 0,
        "the image must not pre-declare a command"
    );
    assert_eq!(
        after.init_cmd,
        FALCON_APPLICATION_INTERFACE_DMEM_MAPPER_V3_CMD_FRTS
    );

    // And the command landed where the mapper said it would.
    let at = after.cmd_in_buffer_offset as usize;
    assert_eq!(&out.dmem[at..at + cmd.len()], &cmd[..]);
    assert_ne!(
        &dmem[at..at + cmd.len()],
        &cmd[..],
        "the command buffer must not already contain the command — that would make \
         the copy unobservable, which is the marker-fill lesson"
    );
}

/// The second FWSEC command goes through the same table. A table that only
/// satisfied FRTS would stop the boot one step later, at `SB`.
#[test]
fn the_same_table_serves_the_sb_command() {
    let img = good();
    let p = ga106();
    let dmem = dmem_of(&img, p);
    let out = patch_interface_data(
        &dmem,
        FALCON_APPLICATION_INTERFACE_DMEM_MAPPER_V3_CMD_SB,
        &fake_cmd_buffer(),
        p.fwsec.interface_offset,
    )
    .expect("SB must walk the same table");
    let after = FalconApplicationInterfaceDmemMapperV3::decode(&out.dmem[out.mapper_at as usize..])
        .unwrap();
    assert_eq!(
        after.init_cmd,
        FALCON_APPLICATION_INTERFACE_DMEM_MAPPER_V3_CMD_SB
    );
}

// ─── Non-vacuity: poison the interface table, watch the walk refuse ──────────

/// Poison the header's `entryCount` to 1 — the driver's own floor.
#[test]
fn one_interface_entry_is_too_few() {
    let img = good();
    let p = ga106();
    let mut dmem = dmem_of(&img, p);
    // `entryCount` is the header's fourth byte.
    dmem[p.fwsec.interface_offset as usize + 3] = 1;
    assert_eq!(
        patch_interface_data(
            &dmem,
            FALCON_APPLICATION_INTERFACE_DMEM_MAPPER_V3_CMD_FRTS,
            &fake_cmd_buffer(),
            p.fwsec.interface_offset,
        ),
        Err(PatchFail::TooFewEntries)
    );
}

/// Poison **both** entries' `id`. That both must be poisoned is itself the proof
/// that the builder emits two mappers rather than one and a filler.
#[test]
fn a_table_with_no_dmemmapper_entry_is_the_failure_the_guest_reported() {
    let img = good();
    let p = ga106();
    let base = p.fwsec.interface_offset as usize + FalconApplicationInterfaceHeaderV1::SIZE;
    let cmd = fake_cmd_buffer();

    // Poisoning only the first still walks: the second is a mapper too.
    let mut one = dmem_of(&img, p);
    one[base] = 0xEE;
    assert!(
        patch_interface_data(
            &one,
            FALCON_APPLICATION_INTERFACE_DMEM_MAPPER_V3_CMD_FRTS,
            &cmd,
            p.fwsec.interface_offset
        )
        .is_ok(),
        "one surviving mapper entry must still satisfy the walk"
    );

    let mut both = one;
    both[base + FalconApplicationInterfaceEntryV1::SIZE] = 0xEE;
    assert_eq!(
        patch_interface_data(
            &both,
            FALCON_APPLICATION_INTERFACE_DMEM_MAPPER_V3_CMD_FRTS,
            &cmd,
            p.fwsec.interface_offset
        ),
        Err(PatchFail::NoDmemMapper),
        "this is `failed to find required interface entry for FWSEC cmd 0x15`"
    );
}

/// An `interfaceOffset` at or past `DMEMLoadSize` is refused before anything is
/// read.
#[test]
fn an_interface_offset_outside_dmem_is_refused() {
    let img = good();
    let p = ga106();
    let dmem = dmem_of(&img, p);
    assert_eq!(
        patch_interface_data(
            &dmem,
            FALCON_APPLICATION_INTERFACE_DMEM_MAPPER_V3_CMD_FRTS,
            &fake_cmd_buffer(),
            p.fwsec.dmem_load_size,
        ),
        Err(PatchFail::InvalidOffset)
    );
}

/// The mapper's `dmemOffset` is bounds-checked against `sizeof(mapper)`, not
/// merely against the section end.
#[test]
fn a_mapper_that_straddles_the_end_of_dmem_is_refused() {
    let img = good();
    let p = ga106();
    let mut dmem = dmem_of(&img, p);
    let straddle = p.fwsec.dmem_load_size - FalconApplicationInterfaceDmemMapperV3::SIZE as u32 + 1;
    let base = p.fwsec.interface_offset as usize + FalconApplicationInterfaceHeaderV1::SIZE;
    for i in 0..usize::from(INTERFACE_ENTRY_COUNT) {
        let at = base + i * FalconApplicationInterfaceEntryV1::SIZE + 4; // `dmemOffset`
        dmem[at..at + 4].copy_from_slice(&straddle.to_le_bytes());
    }
    assert_eq!(
        patch_interface_data(
            &dmem,
            FALCON_APPLICATION_INTERFACE_DMEM_MAPPER_V3_CMD_FRTS,
            &fake_cmd_buffer(),
            p.fwsec.interface_offset,
        ),
        Err(PatchFail::InvalidOffset)
    );
}

// ─── The builder's own DMEM refusals ─────────────────────────────────────────

#[test]
fn refuses_an_interface_table_that_leaves_dmem() {
    let mut p = *ga106();
    p.fwsec.interface_offset = p.fwsec.dmem_load_size - 4;
    assert!(matches!(
        build(&p, VbiosWire::Tu102Bit),
        Err(VbiosError::DmemRegionOutOfBounds { .. })
    ));
}

#[test]
fn refuses_a_mapper_that_leaves_dmem() {
    let mut p = *ga106();
    p.fwsec.dmem_mapper_offset =
        p.fwsec.dmem_load_size - FalconApplicationInterfaceDmemMapperV3::SIZE as u32 + 1;
    assert!(matches!(
        build(&p, VbiosWire::Tu102Bit),
        Err(VbiosError::DmemRegionOutOfBounds { .. })
    ));
}

/// ★ The overrun the DRIVER does not catch. `cmd_in_buffer_offset` is tested
/// against `mappedDataSize`, but the copy's *end* is guarded by the entry's
/// `dmemOffset` instead — so a buffer starting in range and ending out of it
/// walks clean and writes past the section. The builder refuses it here because
/// nothing downstream will.
#[test]
fn refuses_a_command_buffer_that_ends_outside_dmem() {
    let mut p = *ga106();
    p.fwsec.cmd_in_buffer_offset = p.fwsec.dmem_load_size - 4;
    p.fwsec.cmd_in_buffer_size = FWSEC_CMD_BUFFER_MIN;
    assert!(matches!(
        build(&p, VbiosWire::Tu102Bit),
        Err(VbiosError::DmemRegionOutOfBounds { .. })
    ));
}

/// The signature is copied into DMEM **after** the command is patched in, so an
/// overlap silently destroys the command rather than failing anything.
#[test]
fn refuses_a_signature_that_overlaps_the_command_buffer() {
    let mut p = *ga106();
    p.fwsec.pkc_data_offset = p.fwsec.cmd_in_buffer_offset;
    assert!(matches!(
        build(&p, VbiosWire::Tu102Bit),
        Err(VbiosError::DmemRegionsOverlap { .. })
    ));
}

#[test]
fn refuses_a_mapper_that_overlaps_its_own_interface_table() {
    let mut p = *ga106();
    p.fwsec.dmem_mapper_offset = p.fwsec.interface_offset;
    assert!(matches!(
        build(&p, VbiosWire::Tu102Bit),
        Err(VbiosError::DmemRegionsOverlap { .. })
    ));
}

#[test]
fn refuses_a_command_buffer_too_small_for_a_fwsec_command() {
    let mut p = *ga106();
    p.fwsec.cmd_in_buffer_size = FWSEC_CMD_BUFFER_MIN - 1;
    assert_eq!(
        build(&p, VbiosWire::Tu102Bit),
        Err(VbiosError::CmdBufferTooSmall {
            declared: FWSEC_CMD_BUFFER_MIN - 1,
            needed: FWSEC_CMD_BUFFER_MIN,
        })
    );
    // …and one more byte is accepted, so the boundary is where it is claimed.
    p.fwsec.cmd_in_buffer_size = FWSEC_CMD_BUFFER_MIN;
    assert!(build(&p, VbiosWire::Tu102Bit).is_ok());
}

/// Every shipped profile carries a walkable table, not just the one the other
/// tests use. A row added to `VBIOS_PROFILES` cannot silently be one that stops
/// the guest at `s_prepareForFwsec_TU102`.
#[test]
fn every_shipped_profile_carries_a_walkable_interface_table() {
    let mut checked = 0usize;
    for p in VBIOS_PROFILES {
        let img = build(p, VbiosWire::Tu102Bit).expect("builds");
        let out = patch_interface_data(
            &dmem_of(&img, p),
            FALCON_APPLICATION_INTERFACE_DMEM_MAPPER_V3_CMD_FRTS,
            &fake_cmd_buffer(),
            p.fwsec.interface_offset,
        )
        .unwrap_or_else(|e| panic!("profile {} has no walkable table: {e:?}", p.name));
        assert_eq!(out.mapper_at, p.fwsec.dmem_mapper_offset, "{}", p.name);
        checked += 1;
    }
    assert_eq!(checked, VBIOS_PROFILES.len());
    assert!(checked >= 2, "the table must exercise more than one row");
}
