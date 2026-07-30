//! **Synthetic VBIOS generation** — build a ROM image the stock NVIDIA driver's
//! own `kgspExtractVbiosFromRom_TU102` → `kgspParseFwsecUcodeFromVbiosImg` path
//! accepts, from the same profile that drives the emulated device's registers.
//!
//! # Why generate instead of serving a dump
//!
//! Two reasons, and the second is the one that decides it.
//!
//! 1. A dumped ROM needs one-time root on a machine that has the card, which is
//!    an unstable step to ship.
//! 2. ★ **A dumped ROM describes the host's card.** We emulate a *different*
//!    device — our own FB size, our own straps, our own PCI identity — so a dump
//!    can silently disagree with the registers the device answers, and the
//!    disagreement surfaces as an unexplainable failure much later in bring-up.
//!    An image generated from the same [`VbiosProfile`] that drives those
//!    registers **cannot** disagree, by construction.
//!
//! # Why this is bounded: no cryptography is involved
//!
//! The kernel driver performs **no cryptographic verification** of anything in
//! this path. Measured by reading it, at both vendored ogkm tags:
//!
//! * `kernel_gsp_fwsec.c:993` — a plain `portMemCopy` lifts the signature blob
//!   out of the image into a buffer. Nothing hashes it.
//! * `kernel_gsp_frts_tu102.c:355` — `NV_ASSERT_OR_RETURN(pUcode->pSignatures !=
//!   NULL, …)`. That is a **null check**, not a validity check.
//! * `kernel_gsp_frts_tu102.c:397` — one signature is copied to DMEM at
//!   `hsSigDmemAddr` and handed to the falcon.
//!
//! Everything else that *looks* cryptographic is a magic
//! ([`BIT_HEADER_SIGNATURE`] = `"BIT\0"`), a size
//! ([`BCRT30_RSA3K_SIG_SIZE`] = 384), a one-byte checksum (the BIT header), or a
//! bounds check (`portSafeAddU32`, offsets `<= biosSize`).
//!
//! ★ Verification happens **on the falcon**, and in Mode 2 we *are* the falcon.
//! So this module needs **structure, not secrets**: a signature blob of the right
//! size at the declared offset, whose contents nothing outside our control ever
//! inspects. That is the entire reason a synthetic VBIOS is a tractable object.
//!
//! # The real work is offset arithmetic
//!
//! Every offset the driver computes must survive `portSafeAddU32` and land inside
//! `biosSize`, and the descriptor's self-declared sizes must satisfy eight
//! separate inequalities in `s_vbiosFillFlcnUcodeFromDescV3`. [`build`] computes
//! them all and refuses rather than emitting an image that would be rejected —
//! see [`VbiosError`].
//!
//! # What is a table row
//!
//! * **A new GPU / architecture** is one [`VbiosProfile`] appended to
//!   [`VBIOS_PROFILES`]. No logic-crate edit.
//! * **A new driver version** is a [`VbiosWire`] variant plus the `vbios:` field
//!   on that version's [`crate::versions::DriverAbiTable`] row. Today there is
//!   exactly one variant, because the parse path is **byte-identical** at
//!   580.159.04 and 610.43.02 — see [`VbiosWire`].
//!
//! # What this module deliberately does NOT build
//!
//! The **IFR / ROM-directory** prologue. A real card's ROM begins with an IFR
//! block and the PCI expansion-ROM signature sits further in, which is why
//! `kgspExtractVbiosFromRom_TU102` has `s_romImgFindPciHeader_TU102` at all. That
//! whole path runs **only** when the signature is not already at offset 0
//! (`kernel_gsp_vbios_tu102.c:492-503`). A generated image simply puts
//! [`PCI_EXP_ROM_SIGNATURE`] at offset 0, so the IFR path is never entered and
//! none of its constants are needed. This is a case where being synthetic is
//! strictly simpler than being a copy.

use crate::generated::vbios::{
    BCRT30_RSA3K_SIG_SIZE, BIT_DATA_BIOSDATA_BINVER_SIZE_5, BIT_DATA_BIOSDATA_VERSION_2,
    BIT_DATA_FALCON_DATA_V2_SIZE_4, BIT_HEADER_ID, BIT_HEADER_SIGNATURE, BIT_HEADER_SIZE_OFFSET,
    BIT_TOKEN_BIOSDATA, BIT_TOKEN_FALCON_DATA, BIT_TOKEN_V1_00_SIZE_8,
    FALCON_UCODE_DESC_V3_SIZE_44, FALCON_UCODE_ENTRY_APPID_FIRMWARE_SEC_LIC,
    FALCON_UCODE_ENTRY_APPID_FWSEC_DBG, FALCON_UCODE_ENTRY_APPID_FWSEC_PROD,
    FALCON_UCODE_TABLE_ENTRY_V1_SIZE_6, FALCON_UCODE_TABLE_HDR_V1_SIZE_6,
    FALCON_UCODE_TABLE_HDR_V1_VERSION, NV_BCRT_HASH_INFO_BASE_CODE_TYPE_VBIOS_BASE,
    NV_BIT_FALCON_UCODE_DESC_HEADER_VDESC_FLAGS_VERSION,
    NV_BIT_FALCON_UCODE_DESC_HEADER_VDESC_FLAGS_VERSION_AVAILABLE,
    NV_BIT_FALCON_UCODE_DESC_HEADER_VDESC_SIZE, NV_BIT_FALCON_UCODE_DESC_HEADER_VDESC_VERSION,
    NV_BIT_FALCON_UCODE_DESC_HEADER_VDESC_VERSION_V3, NV_PCI_DATA_EXT_REV_11, NV_PCI_DATA_EXT_SIG,
    OFFSETOF_PCI_DATA_EXT_STRUCT_LAST_IMAGE, OFFSETOF_PCI_DATA_EXT_STRUCT_LEN,
    OFFSETOF_PCI_DATA_EXT_STRUCT_REV, OFFSETOF_PCI_DATA_EXT_STRUCT_SIG,
    OFFSETOF_PCI_DATA_EXT_STRUCT_SUBIMAGE_LEN, OFFSETOF_PCI_DATA_STRUCT_CLASS_CODE,
    OFFSETOF_PCI_DATA_STRUCT_CODE_TYPE, OFFSETOF_PCI_DATA_STRUCT_IMAGE_LEN,
    OFFSETOF_PCI_DATA_STRUCT_LAST_IMAGE, OFFSETOF_PCI_DATA_STRUCT_LEN,
    OFFSETOF_PCI_DATA_STRUCT_SIG, OFFSETOF_PCI_DATA_STRUCT_VENDOR_ID,
    OFFSETOF_PCI_EXP_ROM_PCI_DATA_STRUCT_PTR, OFFSETOF_PCI_EXP_ROM_SIG, PCI_DATA_STRUCT_SIGNATURE,
    PCI_EXP_ROM_SIGNATURE, PCI_LAST_IMAGE, PCI_ROM_IMAGE_BLOCK_SIZE,
};

/// The maximum VBIOS image size the driver will accept, in bytes.
///
/// ★ **A declared constant, not a scanned one** — and the only one in this
/// module. `s_getBaseBiosMaxSize_TU102` is a *function returning a literal*
/// (`kernel_gsp_vbios_tu102.c:422-429`: `return 0x100000; // 1 MB`), not a
/// `#define`, so there is nothing for the generator's `#define` scanner to read.
/// The driver both seeds `src.maxOffset` with it and rejects any image claiming
/// more (`:519-524`, "expansion ROM has exceedingly large size").
///
/// This is the honest place a bolt-on could start: if a future architecture
/// changes the cap, this needs a per-[`VbiosWire`] value rather than a constant.
/// It is one number with one citation, which is why it is acceptable today.
pub const BIOS_MAX_SIZE: usize = 0x0010_0000;

/// The falcon ucode payload's alignment, in bytes.
///
/// `s_vbiosFillFlcnUcodeFromDescV3` computes `pUcode->size =
/// RM_ALIGN_UP(pDescV3->StoredSize, 256)` (`kernel_gsp_fwsec.c:912`) and then
/// bounds-checks everything against that rounded value, so a `StoredSize` that is
/// not already a multiple of 256 silently widens every check. [`build`] rounds up
/// itself and emits that many bytes, so the declared and actual sizes agree.
pub const UCODE_ALIGN: u32 = 256;

/// The XOR key the opaque falcon ucode payload is filled with.
///
/// The payload's *content* is never inspected by the driver — it is copied into
/// a memdesc and handed to the falcon, which in Mode 2 is us. But its *position*
/// is forced (`descOffset + descSize`), and a zero-filled payload is
/// indistinguishable from the zero padding either side of it, so a
/// wrongly-placed payload would be undetectable. Filling it with
/// `UCODE_MARKER ^ (i % 251)` makes the placement observable to a test and makes
/// a stray copy identifiable in a trace.
pub const UCODE_MARKER: u8 = 0xA5;

/// Which VBIOS parse path a driver version speaks.
///
/// # There is one variant, and that is a measurement
///
/// The four files that define this entire path are **byte-identical** at both
/// vendored ogkm tags — 580.159.04 and 610.43.02:
///
/// | file | `diff` |
/// |---|---|
/// | `src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_vbios_tu102.c` | empty |
/// | `src/nvidia/src/kernel/gpu/gsp/kernel_gsp_fwsec.c` | empty |
/// | `src/nvidia/inc/kernel/platform/pci_exp_table.h` | empty |
/// | `src/common/inc/swref/published/turing/tu102/dev_bus.h` | empty |
///
/// and the generator run against each tag emits a `generated/vbios.rs` that
/// differs only in its provenance banner. Contrast the *rest* of the generated
/// tree over the same two tags: `nvos.rs` 122 changed lines, `ctrl.rs` 34,
/// `classes.rs` 22, `rpc.rs` 2. So the emptiness here is a property of this path,
/// not of the comparison.
///
/// A second variant is therefore a **table row** the day a version diverges, and
/// the enum exists so that day is a data change rather than a redesign.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VbiosWire {
    /// PCI expansion-ROM container → BIT table → falcon ucode table → FWSEC
    /// descriptor V2/V3. Introduced with Turing (`kgspExtractVbiosFromRom_TU102`)
    /// and unchanged through 610.43.02.
    Tu102Bit,
}

/// Everything about a *device* that a synthetic VBIOS has to state.
///
/// ★ **This is the table row.** Adding a GPU or an architecture is appending one
/// of these to [`VBIOS_PROFILES`]; no logic in this crate or any other changes.
/// Every field is either an identity the emulated device also reports through its
/// registers/config space (so the two cannot disagree) or a FWSEC descriptor
/// value the driver copies without interpreting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VbiosProfile {
    /// Human-readable chip name, for diagnostics only. Never branched on.
    pub name: &'static str,
    /// PCI vendor ID, written into the PCIR structure. Must equal what the
    /// emulated device's config space reports.
    pub pci_vendor_id: u16,
    /// PCI device ID — also the lookup key ([`profile_for_device_id`]).
    pub pci_device_id: u16,
    /// PCI class code, low byte first, as the PCIR structure stores it.
    pub pci_class_code: [u8; 3],
    /// VBIOS binary version, reported back through
    /// `pVbiosVersionCombined` (`kernel_gsp_fwsec.c:551`). Cosmetic: it reaches
    /// `nvidia-smi`, never a decision.
    pub vbios_version: u32,
    /// OEM version byte, spliced into the low 8 bits of the combined version.
    pub vbios_oem_version: u8,
    /// The FWSEC ucode descriptor this profile declares.
    pub fwsec: FwsecProfile,
}

/// The FWSEC falcon ucode descriptor's self-declared geometry.
///
/// These are the numbers `s_vbiosFillFlcnUcodeFromDescV3` reads and bounds-checks
/// (`kernel_gsp_fwsec.c:908-978`). None of them is verified against the ucode
/// payload's *content* — the payload is opaque bytes the driver copies into a
/// memdesc and hands to the falcon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FwsecProfile {
    /// `IMEMLoadSize` — bytes of instruction memory the falcon loads.
    pub imem_load_size: u32,
    /// `IMEMPhysBase`.
    pub imem_phys_base: u32,
    /// `IMEMVirtBase`.
    pub imem_virt_base: u32,
    /// `DMEMLoadSize` — bytes of data memory.
    pub dmem_load_size: u32,
    /// `DMEMPhysBase`.
    pub dmem_phys_base: u32,
    /// `PKCDataOffset` — becomes `hsSigDmemAddr`, the DMEM address the signature
    /// is copied to. Bounds-checked relative to the *data* section.
    pub pkc_data_offset: u32,
    /// `InterfaceOffset`.
    pub interface_offset: u32,
    /// `UcodeId`.
    pub ucode_id: u8,
    /// `EngineIdMask`.
    pub engine_id_mask: u16,
    /// `SignatureCount` — how many [`BCRT30_RSA3K_SIG_SIZE`]-byte signatures
    /// follow the 44-byte descriptor. Drives `descSize`, hence
    /// `signaturesTotalSize`.
    pub signature_count: u8,
    /// `SignatureVersions` bitmask.
    pub signature_versions: u16,
}

/// Why a profile could not be turned into an image.
///
/// Every variant is a check the *driver* performs; refusing here means the guest
/// would have refused there. MISS = FAULT — [`build`] never silently clamps a
/// value to make it fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VbiosError {
    /// No [`VbiosProfile`] in [`VBIOS_PROFILES`] has this PCI device ID. There is
    /// no nearest-neighbour fallback by design.
    NoProfileForDevice {
        /// The PCI device ID that was asked for.
        device_id: u16,
    },
    /// `signature_count` is 0. `s_vbiosFillFlcnUcodeFromDescV3` would compute
    /// `signaturesTotalSize = 0` and `portMemAllocNonPaged(0)`, and
    /// `kernel_gsp_frts_tu102.c:355`'s null check is the only thing standing
    /// between that and a null deref — so an image must declare at least one.
    NoSignatures,
    /// `44 + signature_count * 384` does not fit the 16-bit `vDesc` size field
    /// ([`NV_BIT_FALCON_UCODE_DESC_HEADER_VDESC_SIZE`]), so the descriptor could
    /// not state its own length.
    DescriptorTooLarge {
        /// The descriptor size that did not fit.
        desc_size: usize,
    },
    /// `imem_load_size + dmem_load_size` exceeds the aligned ucode size, so
    /// `dataOffset + dmemSize > size` (`kernel_gsp_fwsec.c:961-965`).
    UcodeSectionsOverflow {
        /// `imem_load_size + dmem_load_size`.
        needed: u32,
        /// `RM_ALIGN_UP(stored_size, 256)`.
        available: u32,
    },
    /// The signature would not fit in the data section at `pkc_data_offset`:
    /// `dataOffset + hsSigDmemAddr + sigSize > size`
    /// (`kernel_gsp_fwsec.c:968-978`).
    SignatureOutOfUcode {
        /// `imem_load_size + pkc_data_offset + 384`.
        sig_end: u32,
        /// `RM_ALIGN_UP(stored_size, 256)`.
        available: u32,
    },
    /// The finished image exceeds [`BIOS_MAX_SIZE`], which the driver rejects
    /// with "expansion ROM has exceedingly large size".
    ImageTooLarge {
        /// Size the image would have had.
        size: usize,
    },
}

impl core::fmt::Display for VbiosError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoProfileForDevice { device_id } => write!(
                f,
                "no synthetic-VBIOS profile for PCI device {device_id:#06x}"
            ),
            Self::NoSignatures => write!(
                f,
                "profile declares signature_count=0; the descriptor must carry at least one"
            ),
            Self::DescriptorTooLarge { desc_size } => write!(
                f,
                "descriptor size {desc_size} does not fit the 16-bit vDesc size field"
            ),
            Self::UcodeSectionsOverflow { needed, available } => write!(
                f,
                "imem+dmem is {needed} bytes but the aligned ucode is only {available}"
            ),
            Self::SignatureOutOfUcode { sig_end, available } => write!(
                f,
                "signature ends at {sig_end} but the aligned ucode is only {available}"
            ),
            Self::ImageTooLarge { size } => {
                write!(
                    f,
                    "image is {size} bytes, over the {BIOS_MAX_SIZE}-byte cap"
                )
            }
        }
    }
}

impl core::error::Error for VbiosError {}

/// The known device profiles.
///
/// ★ **Adding an architecture is appending a row here.** Nothing else.
pub static VBIOS_PROFILES: &[VbiosProfile] = &[
    // GA106 — the part the C research artifact ran against, and the identity the
    // bench's `x-nvidia-identity` experiment claims. Its numbers are ours: the
    // FWSEC geometry below is a *generated* geometry that satisfies the driver's
    // inequalities, NOT a transcription of any real card's ROM.
    VbiosProfile {
        name: "GA106",
        pci_vendor_id: 0x10DE,
        pci_device_id: 0x2504,
        // 0x030000 = VGA-compatible display controller, stored low byte first.
        pci_class_code: [0x00, 0x00, 0x03],
        vbios_version: 0x9418_0000,
        vbios_oem_version: 0x00,
        fwsec: FwsecProfile {
            imem_load_size: 0x1000,
            imem_phys_base: 0,
            imem_virt_base: 0,
            dmem_load_size: 0x1000,
            dmem_phys_base: 0,
            // Inside the 0x1000-byte data section with room for a 384-byte
            // signature: 0x800 + 384 = 0x980 <= 0x1000.
            pkc_data_offset: 0x0800,
            interface_offset: 0,
            ucode_id: 0x01,
            engine_id_mask: 0x0001,
            signature_count: 1,
            signature_versions: 0x0001,
        },
    },
];

/// Look a profile up by PCI device ID.
///
/// # Errors
///
/// [`VbiosError::NoProfileForDevice`] if no row matches. There is deliberately no
/// fallback: serving a GA106 ROM to a device claiming to be something else is
/// exactly the host/guest disagreement this module exists to prevent.
pub fn profile_for_device_id(device_id: u16) -> Result<&'static VbiosProfile, VbiosError> {
    VBIOS_PROFILES
        .iter()
        .find(|p| p.pci_device_id == device_id)
        .ok_or(VbiosError::NoProfileForDevice { device_id })
}

/// A little-endian byte canvas addressed by absolute offset.
///
/// Writes grow the image as needed and never panic on a short buffer, so layout
/// arithmetic mistakes show up as a failed parse in the tests rather than as an
/// index panic in production.
struct Canvas {
    bytes: Vec<u8>,
}

impl Canvas {
    fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    fn grow_to(&mut self, end: usize) {
        if self.bytes.len() < end {
            self.bytes.resize(end, 0);
        }
    }

    fn put(&mut self, at: usize, src: &[u8]) {
        self.grow_to(at + src.len());
        self.bytes[at..at + src.len()].copy_from_slice(src);
    }

    fn u8(&mut self, at: usize, v: u8) {
        self.put(at, &[v]);
    }

    fn u16(&mut self, at: usize, v: u16) {
        self.put(at, &v.to_le_bytes());
    }

    fn u32(&mut self, at: usize, v: u32) {
        self.put(at, &v.to_le_bytes());
    }
}

/// Round `v` up to a multiple of `to` (a power of two).
const fn align_up(v: usize, to: usize) -> usize {
    v.div_ceil(to) * to
}

/// Build a synthetic VBIOS image for `profile`.
///
/// The result is a complete PCI expansion ROM: signature at offset 0, one
/// **base**-code-type image, a BIT table, a falcon ucode table naming FWSEC three
/// ways, a V3 FWSEC descriptor, its signature blob, and the ucode payload —
/// padded to a whole number of [`PCI_ROM_IMAGE_BLOCK_SIZE`] blocks.
///
/// # The FWSEC entry is declared under three application IDs
///
/// ★ Read `kernel_gsp_fwsec.c:620-625`'s skip condition carefully:
///
/// ```text
/// if (appId != FIRMWARE_SEC_LIC &&
///     (( bUseDebugFwsec && appId != FWSEC_DBG) ||
///      (!bUseDebugFwsec && appId != FWSEC_PROD)))  continue;
/// ```
///
/// An entry whose appId is [`FALCON_UCODE_ENTRY_APPID_FIRMWARE_SEC_LIC`] (`0x05`)
/// short-circuits the `&&` and matches **regardless of debug mode**. Whether the
/// emulated GPU reads as debug-fused or prod-fused depends on
/// `kgspIsDebugModeEnabled_HAL`, which reads a fuse register we have not yet
/// modelled — so relying on that answer would be guessing. Emitting `0x05` first,
/// then the debug and prod IDs pointing at the same descriptor, makes the image
/// correct under **either** answer. The driver returns on the first match.
///
/// # Errors
///
/// See [`VbiosError`]. Every variant mirrors a check the driver itself performs.
pub fn build(profile: &VbiosProfile, wire: VbiosWire) -> Result<Vec<u8>, VbiosError> {
    let VbiosWire::Tu102Bit = wire;
    let fw = &profile.fwsec;

    // ── Descriptor and payload geometry, validated against the driver's own
    //    inequalities before a single byte is written. ──────────────────────
    if fw.signature_count == 0 {
        return Err(VbiosError::NoSignatures);
    }
    let sig_total = usize::from(fw.signature_count) * BCRT30_RSA3K_SIG_SIZE;
    let desc_size = FALCON_UCODE_DESC_V3_SIZE_44 + sig_total;
    // The descriptor states its own size in a 16-bit DRF field.
    let Ok(desc_size_u32) = u32::try_from(desc_size) else {
        return Err(VbiosError::DescriptorTooLarge { desc_size });
    };
    if !NV_BIT_FALCON_UCODE_DESC_HEADER_VDESC_SIZE.fits(desc_size_u32) {
        return Err(VbiosError::DescriptorTooLarge { desc_size });
    }

    // `pUcode->size = RM_ALIGN_UP(StoredSize, 256)`, and every section check is
    // against the ROUNDED value. Declare a StoredSize that is already aligned so
    // the declared and effective sizes are the same number.
    let sections = fw.imem_load_size.saturating_add(fw.dmem_load_size);
    let ucode_size =
        u32::try_from(align_up(sections as usize, UCODE_ALIGN as usize)).map_err(|_| {
            VbiosError::UcodeSectionsOverflow {
                needed: sections,
                available: u32::MAX,
            }
        })?;
    // `dataOffset = imemSize`, so `dataOffset + dmemSize <= size`.
    if sections > ucode_size {
        return Err(VbiosError::UcodeSectionsOverflow {
            needed: sections,
            available: ucode_size,
        });
    }
    // `sigDataOffset = dataOffset + hsSigDmemAddr`, then `+ sigSize <= size`.
    let sig_end = fw
        .imem_load_size
        .saturating_add(fw.pkc_data_offset)
        .saturating_add(BCRT30_RSA3K_SIG_SIZE as u32);
    if sig_end > ucode_size {
        return Err(VbiosError::SignatureOutOfUcode {
            sig_end,
            available: ucode_size,
        });
    }

    // ── Layout. Everything below is derived; no offset is a magic number. ────
    let mut c = Canvas::new();

    // The PCIR structure follows the 0x1A-byte expansion-ROM header. 4-align it
    // so the driver's dword reads never straddle awkwardly.
    let pcir = align_up(OFFSETOF_PCI_EXP_ROM_PCI_DATA_STRUCT_PTR + 2, 4);
    let pcir_len = 0x18usize; // the PCI 3.0 Data Structure length
    // `s_locateExpansionRoms` looks for the NPDE at (pcir + len + 0xF) & ~0xF.
    let npde = (pcir + pcir_len + 0xF) & !0xF;
    let npde_len = 0x0Cusize;

    let bit = align_up(npde + npde_len, 0x10);
    let bit_header_size = 12usize; // BIT_HEADER_V1_00 = "1w1d1w4b"
    let token_size = usize::from(BIT_TOKEN_V1_00_SIZE_8);
    let tokens = bit + bit_header_size;
    let token_count = 2usize; // BIOSDATA, FALCON_DATA

    let binver = align_up(tokens + token_count * token_size, 4);
    let binver_len = 5usize; // "1d1b"
    // The falcon-data token's DataPtr points at a single u32 holding the table's
    // address (`BIT_DATA_FALCON_DATA_V2`).
    let falcon_data = align_up(binver + binver_len, 4);
    let ucode_table = align_up(falcon_data + 4, 0x10);
    let hdr_size = usize::from(FALCON_UCODE_TABLE_HDR_V1_SIZE_6);
    let entry_size = usize::from(FALCON_UCODE_TABLE_ENTRY_V1_SIZE_6);
    let app_ids = [
        FALCON_UCODE_ENTRY_APPID_FIRMWARE_SEC_LIC,
        FALCON_UCODE_ENTRY_APPID_FWSEC_DBG,
        FALCON_UCODE_ENTRY_APPID_FWSEC_PROD,
    ];
    let entries = ucode_table + hdr_size;

    let desc = align_up(entries + app_ids.len() * entry_size, 0x10);
    // ★ FORCED, not chosen: `imageOffset = descOffset + descSize`
    // (`kernel_gsp_fwsec.c:937`). The payload must begin immediately after the
    // descriptor and its signatures, with no alignment gap.
    let ucode = desc + desc_size;
    let image_end = ucode + ucode_size as usize;

    let blocks = align_up(image_end, PCI_ROM_IMAGE_BLOCK_SIZE) / PCI_ROM_IMAGE_BLOCK_SIZE;
    let image_size = blocks * PCI_ROM_IMAGE_BLOCK_SIZE;
    if image_size > BIOS_MAX_SIZE {
        return Err(VbiosError::ImageTooLarge { size: image_size });
    }
    let Ok(blocks_u16) = u16::try_from(blocks) else {
        return Err(VbiosError::ImageTooLarge { size: image_size });
    };

    // ── The PCI expansion-ROM header ─────────────────────────────────────────
    c.u16(OFFSETOF_PCI_EXP_ROM_SIG, PCI_EXP_ROM_SIGNATURE);
    c.u16(
        OFFSETOF_PCI_EXP_ROM_PCI_DATA_STRUCT_PTR,
        u16::try_from(pcir).unwrap_or(0),
    );

    // ── The PCIR structure ───────────────────────────────────────────────────
    c.u32(
        pcir + OFFSETOF_PCI_DATA_STRUCT_SIG,
        PCI_DATA_STRUCT_SIGNATURE,
    );
    c.u16(
        pcir + OFFSETOF_PCI_DATA_STRUCT_VENDOR_ID,
        profile.pci_vendor_id,
    );
    c.u16(
        pcir + OFFSETOF_PCI_DATA_STRUCT_VENDOR_ID + 2,
        profile.pci_device_id,
    );
    c.u16(
        pcir + OFFSETOF_PCI_DATA_STRUCT_LEN,
        u16::try_from(pcir_len).unwrap_or(0),
    );
    c.put(
        pcir + OFFSETOF_PCI_DATA_STRUCT_CLASS_CODE,
        &profile.pci_class_code,
    );
    c.u16(pcir + OFFSETOF_PCI_DATA_STRUCT_IMAGE_LEN, blocks_u16);
    // A single image, so it is the BASE one. `s_locateExpansionRoms` then leaves
    // `extRomOffset` at 0 and yields `expansionRomOffset = 0`, which is why every
    // `expansionRomOffset + ptr` in the FWSEC parser is just `ptr` here.
    c.u8(
        pcir + OFFSETOF_PCI_DATA_STRUCT_CODE_TYPE,
        NV_BCRT_HASH_INFO_BASE_CODE_TYPE_VBIOS_BASE,
    );
    c.u8(pcir + OFFSETOF_PCI_DATA_STRUCT_LAST_IMAGE, PCI_LAST_IMAGE);

    // ── The NPDE extension ───────────────────────────────────────────────────
    // Present and consistent with the PCIR: when it is found, `subImgLen` from
    // here OVERRIDES `IMAGE_LEN` and is what advances the walk.
    c.u32(npde + OFFSETOF_PCI_DATA_EXT_STRUCT_SIG, NV_PCI_DATA_EXT_SIG);
    c.u16(
        npde + OFFSETOF_PCI_DATA_EXT_STRUCT_REV,
        NV_PCI_DATA_EXT_REV_11,
    );
    c.u16(
        npde + OFFSETOF_PCI_DATA_EXT_STRUCT_LEN,
        u16::try_from(npde_len).unwrap_or(0),
    );
    c.u16(npde + OFFSETOF_PCI_DATA_EXT_STRUCT_SUBIMAGE_LEN, blocks_u16);
    c.u8(
        npde + OFFSETOF_PCI_DATA_EXT_STRUCT_LAST_IMAGE,
        PCI_LAST_IMAGE,
    );

    // ── The BIT header ───────────────────────────────────────────────────────
    c.u16(bit, BIT_HEADER_ID);
    c.u32(bit + 2, BIT_HEADER_SIGNATURE);
    c.u16(bit + 6, 0x0100); // BCD_Version 1.00
    c.u8(
        bit + BIT_HEADER_SIZE_OFFSET,
        u8::try_from(bit_header_size).unwrap_or(0),
    );
    c.u8(bit + 9, BIT_TOKEN_V1_00_SIZE_8);
    c.u8(bit + 10, u8::try_from(token_count).unwrap_or(0));
    c.u8(bit + 11, 0); // checksum, filled in below

    // ── BIT tokens (8-byte form: "2b1w1d") ──────────────────────────────────
    let put_token = |c: &mut Canvas, idx: usize, id: u8, ver: u8, size: u16, ptr: u32| {
        let t = tokens + idx * token_size;
        c.u8(t, id);
        c.u8(t + 1, ver);
        c.u16(t + 2, size);
        c.u32(t + 4, ptr);
    };
    // BIOSDATA: DataSize must be STRICTLY greater than BINVER_SIZE_5 for the
    // version to be read at all (`kernel_gsp_fwsec.c:538`).
    put_token(
        &mut c,
        0,
        BIT_TOKEN_BIOSDATA,
        BIT_DATA_BIOSDATA_VERSION_2,
        BIT_DATA_BIOSDATA_BINVER_SIZE_5 + 1,
        u32::try_from(binver).unwrap_or(0),
    );
    // FALCON_DATA: DataVersion must be exactly 2 (`:555-560`).
    put_token(
        &mut c,
        1,
        BIT_TOKEN_FALCON_DATA,
        2,
        BIT_DATA_FALCON_DATA_V2_SIZE_4,
        u32::try_from(falcon_data).unwrap_or(0),
    );

    // BIT header checksum: the low byte of the sum over `HeaderSize` bytes must
    // be zero (`kernel_gsp_fwsec.c:431-441`).
    let sum: u8 = c.bytes[bit..bit + bit_header_size]
        .iter()
        .fold(0u8, |a, b| a.wrapping_add(*b));
    c.u8(bit + 11, 0u8.wrapping_sub(sum));

    // ── BIOSDATA payload ("1d1b") ────────────────────────────────────────────
    c.u32(binver, profile.vbios_version);
    c.u8(binver + 4, profile.vbios_oem_version);

    // ── BIT_DATA_FALCON_DATA_V2: one u32 pointing at the ucode table ─────────
    c.u32(falcon_data, u32::try_from(ucode_table).unwrap_or(0));

    // ── The falcon ucode table ───────────────────────────────────────────────
    c.u8(ucode_table, FALCON_UCODE_TABLE_HDR_V1_VERSION);
    c.u8(ucode_table + 1, u8::try_from(hdr_size).unwrap_or(0));
    c.u8(ucode_table + 2, u8::try_from(entry_size).unwrap_or(0));
    c.u8(ucode_table + 3, u8::try_from(app_ids.len()).unwrap_or(0));
    c.u8(
        ucode_table + 4,
        u8::try_from(NV_BIT_FALCON_UCODE_DESC_HEADER_VDESC_VERSION_V3).unwrap_or(0),
    );
    c.u8(
        ucode_table + 5,
        u8::try_from(FALCON_UCODE_DESC_V3_SIZE_44).unwrap_or(0),
    );
    for (i, app) in app_ids.iter().enumerate() {
        let e = entries + i * entry_size;
        c.u8(e, *app);
        c.u8(e + 1, 0); // TargetID — read but never tested on this path
        c.u32(e + 2, u32::try_from(desc).unwrap_or(0));
    }

    // ── The FWSEC descriptor, V3 ("9d1w2b2w" = 44 bytes) ─────────────────────
    let vdesc = NV_BIT_FALCON_UCODE_DESC_HEADER_VDESC_FLAGS_VERSION
        .set(NV_BIT_FALCON_UCODE_DESC_HEADER_VDESC_FLAGS_VERSION_AVAILABLE)
        | NV_BIT_FALCON_UCODE_DESC_HEADER_VDESC_VERSION
            .set(NV_BIT_FALCON_UCODE_DESC_HEADER_VDESC_VERSION_V3)
        | NV_BIT_FALCON_UCODE_DESC_HEADER_VDESC_SIZE.set(desc_size_u32);
    c.u32(desc, vdesc);
    c.u32(desc + 4, ucode_size); // StoredSize (already 256-aligned)
    c.u32(desc + 8, fw.pkc_data_offset);
    c.u32(desc + 12, fw.interface_offset);
    c.u32(desc + 16, fw.imem_phys_base);
    c.u32(desc + 20, fw.imem_load_size);
    c.u32(desc + 24, fw.imem_virt_base);
    c.u32(desc + 28, fw.dmem_phys_base);
    c.u32(desc + 32, fw.dmem_load_size);
    c.u16(desc + 36, fw.engine_id_mask);
    c.u8(desc + 38, fw.ucode_id);
    c.u8(desc + 39, fw.signature_count);
    c.u16(desc + 40, fw.signature_versions);
    c.u16(desc + 42, 0); // Reserved

    // ── The signature blob ───────────────────────────────────────────────────
    // Structure, not secret: nothing outside our control ever reads these bytes.
    // They are filled with a recognisable, self-describing pattern rather than
    // zeros so that a stray copy of them is identifiable in a trace — and
    // deliberately NOT with anything that could form a second BIT header (see
    // the `exactly_one_bit_header_candidate` test).
    for i in 0..sig_total {
        c.u8(desc + FALCON_UCODE_DESC_V3_SIZE_44 + i, (i % 251) as u8);
    }

    // ── The ucode payload ────────────────────────────────────────────────────
    // Opaque to the driver: copied into a memdesc and handed to the falcon, which
    // in Mode 2 is us.
    //
    // ★ Filled with a marker pattern rather than left as zeros, and that is not
    // cosmetic. The payload's position is FORCED to `descOffset + descSize`
    // (`kernel_gsp_fwsec.c:937`), but a zero-filled payload is byte-identical to
    // the zero padding around it, so nothing — no test, and no consumer — could
    // detect the payload being written to the wrong offset. A distinguishable
    // pattern makes that placement observable, which is what
    // `tests::the_ucode_payload_sits_immediately_after_the_signatures` checks.
    // (Measured: with a zero payload, deliberately shifting this offset by 16
    // bytes left all 25 tests green.)
    for i in 0..ucode_size as usize {
        c.u8(ucode + i, UCODE_MARKER ^ (i % 251) as u8);
    }

    // ── Pad to a whole number of 512-byte blocks ─────────────────────────────
    c.grow_to(image_size);
    debug_assert_eq!(c.bytes.len(), image_size);
    Ok(c.bytes)
}

#[cfg(test)]
mod tests;
