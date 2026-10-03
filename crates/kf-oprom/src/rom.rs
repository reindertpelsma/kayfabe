// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! The PCI expansion ROM layout: image headers and the PCI data structure (PCIR).
//!
//! References: PCI Firmware Specification 3.x §5.1 (the ROM header and PCIR), UEFI 2.x §14.4.2 (the
//! EFI image header that replaces the legacy header's tail). The consumer kf3 targets is EDK2's
//! `PciBusDxe` (`MdeModulePkg/Bus/Pci/PciBusDxe/PciOptionRomSupport.c`, edk2-stable202408 as shipped
//! in QEMU 10.2.4's `roms/edk2`), whose checks [`images`] mirrors:
//! - `PcirOffset` non-zero and a multiple of 4, the PCIR inside the ROM (`:344-346`, `:471-478`);
//! - an image's `ImageLength` blocks inside the ROM (`:497`), walked until the last-image bit (`:509`);
//! - for an EFI image: `EfiSignature == 0x0EF1`, `InitializationSize <= ImageLength` and
//!   `EfiImageHeaderOffset < InitializationSize`; the bytes handed to `LoadImage` are
//!   `[EfiImageHeaderOffset, InitializationSize * 512)` (`:76-90`).
//!
//! ⚠ A legacy (code type 0) image keeps its own layout after offset 2 — an 8-bit size and an entry
//! jump — so the EFI fields are decoded only for code type 3.

use core::fmt;

/// Bytes per ROM block: `ImageLength` and `InitializationSize` count these.
pub const BLOCK: usize = 512;
/// The ROM header signature, `55 AA`.
pub const ROM_SIGNATURE: u16 = 0xAA55;
/// The EFI image header's `EfiSignature`.
pub const EFI_SIGNATURE: u32 = 0x0EF1;
/// The PCI data structure's signature.
pub const PCIR_SIGNATURE: [u8; 4] = *b"PCIR";
/// `CodeType` of a legacy x86 (PC-AT) image.
pub const CODE_TYPE_PCAT: u8 = 0;
/// `CodeType` of an EFI image.
pub const CODE_TYPE_EFI: u8 = 3;
/// `Indicator` bit 7: this is the last image in the ROM.
pub const INDICATOR_LAST: u8 = 0x80;
/// PE `Subsystem` of an EFI application (EDK2 skips these in a ROM, `PciOptionRomSupport.c:701`).
pub const SUBSYSTEM_EFI_APPLICATION: u16 = 10;
/// PE `Subsystem` of an EFI boot-service driver — what kf3's GOP is.
pub const SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER: u16 = 11;
/// PE `Subsystem` of an EFI runtime driver (EDK2 accepts it in a ROM too).
pub const SUBSYSTEM_EFI_RUNTIME_DRIVER: u16 = 12;
/// PE `Machine` / `EfiMachineType` for x86-64.
pub const MACHINE_X64: u16 = 0x8664;
/// PE `Machine` / `EfiMachineType` for AArch64 (`IMAGE_FILE_MACHINE_ARM64`).
pub const MACHINE_AA64: u16 = 0xAA64;

/// Offset of `EfiSignature` in an EFI image header.
pub const OFF_EFI_SIGNATURE: usize = 0x04;
/// Offset of `EfiSubsystem`.
pub const OFF_EFI_SUBSYSTEM: usize = 0x08;
/// Offset of `EfiMachineType`.
pub const OFF_EFI_MACHINE: usize = 0x0A;
/// Offset of `CompressionType`.
pub const OFF_COMPRESSION: usize = 0x0C;
/// Offset of `EfiImageHeaderOffset`.
pub const OFF_IMAGE_HEADER_OFFSET: usize = 0x16;
/// Offset of `PcirOffset` (the same in the legacy and the EFI header).
pub const OFF_PCIR_OFFSET: usize = 0x18;
/// Length of a PCIR at revision 3 (adds `MaxRuntimeImageLength`, the config-utility and DMTF CLP
/// pointers to revision 0's 0x18).
pub const PCIR_LEN_REV3: u16 = 0x1C;
/// Length of a PCIR at revision 0.
pub const PCIR_LEN_REV0: u16 = 0x18;

/// A PCI class code as the three bytes PCIR carries (`ClassCode[0..3]` = prog-if, sub-class, base).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassCode {
    /// Base class (0x03 = display controller).
    pub base: u8,
    /// Sub-class (0x00 = VGA-compatible, 0x02 = 3D controller).
    pub sub: u8,
    /// Programming interface.
    pub prog_if: u8,
}

impl ClassCode {
    /// The base class of every display controller.
    pub const DISPLAY_BASE: u8 = 0x03;

    /// The class from PCI config space's 24-bit class register (`base << 16 | sub << 8 | prog_if`).
    #[must_use]
    pub fn from_u24(v: u32) -> ClassCode {
        ClassCode {
            base: (v >> 16) as u8,
            sub: (v >> 8) as u8,
            prog_if: v as u8,
        }
    }

    /// The three bytes in PCIR order.
    #[must_use]
    pub fn pcir_bytes(&self) -> [u8; 3] {
        [self.prog_if, self.sub, self.base]
    }
}

/// `CompressionType` of an EFI image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    /// 0: the image is stored as-is. The only value [`crate::pack`] emits.
    None,
    /// 1: EFI (Tiano) compression — what QEMU's iPXE ROMs use.
    Efi,
    /// Any other value; EDK2 does not load such an image (`PciOptionRomSupport.c:80`).
    Other(u16),
}

impl Compression {
    fn from_u16(v: u16) -> Compression {
        match v {
            0 => Compression::None,
            1 => Compression::Efi,
            x => Compression::Other(x),
        }
    }
}

/// The PCI data structure of one image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pcir {
    /// Offset of the structure from the image start (`PcirOffset`).
    pub offset: u16,
    /// Vendor id.
    pub vendor: u16,
    /// Device id.
    pub device: u16,
    /// `DeviceListOffset` at revision 3; reserved (and usually 0) at revision 0.
    pub device_list: u16,
    /// Structure length in bytes.
    pub len: u16,
    /// Structure revision (0 or 3 accepted).
    pub revision: u8,
    /// Class code.
    pub class: ClassCode,
    /// `ImageLength`, in [`BLOCK`]s.
    pub image_blocks: u16,
    /// `CodeRevision`.
    pub code_revision: u16,
    /// `CodeType`.
    pub code_type: u8,
    /// `Indicator`.
    pub indicator: u8,
}

/// The EFI-specific header fields of a code-type-3 image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EfiHeader {
    /// `InitializationSize`, in [`BLOCK`]s.
    pub init_blocks: u16,
    /// `EfiSubsystem`.
    pub subsystem: u16,
    /// `EfiMachineType`.
    pub machine: u16,
    /// `CompressionType`.
    pub compression: Compression,
    /// `EfiImageHeaderOffset`: where the (possibly compressed) PE starts, from the image start.
    pub image_header_offset: u16,
}

/// One image of a ROM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Image<'a> {
    /// Offset of the image in the ROM.
    pub offset: usize,
    /// The image's bytes: `ImageLength` blocks from `offset`.
    pub bytes: &'a [u8],
    /// Its PCI data structure.
    pub pcir: Pcir,
    /// The EFI header, for a code-type-3 image.
    pub efi: Option<EfiHeader>,
}

impl<'a> Image<'a> {
    /// Whether the last-image bit is set.
    #[must_use]
    pub fn is_last(&self) -> bool {
        self.pcir.indicator & INDICATOR_LAST != 0
    }

    /// The bytes EDK2 would hand to `LoadImage`: `[EfiImageHeaderOffset, InitializationSize * 512)`.
    /// `None` for a non-EFI image. Compressed images come back compressed (see [`EfiHeader`]).
    #[must_use]
    pub fn efi_payload(&self) -> Option<&'a [u8]> {
        let efi = self.efi?;
        let start = usize::from(efi.image_header_offset);
        let end = usize::from(efi.init_blocks) * BLOCK;
        self.bytes.get(start..end)
    }
}

/// Why a ROM did not parse. Every arm names the image offset it refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// The ROM holds no image at all.
    Empty,
    /// No `55 AA` at an image start.
    BadSignature {
        /// Image offset.
        at: usize,
    },
    /// `PcirOffset` is 0, not a multiple of 4, or the structure runs past the image.
    BadPcirOffset {
        /// Image offset.
        at: usize,
        /// The offending `PcirOffset`.
        pcir: u16,
    },
    /// No `PCIR` signature where `PcirOffset` points.
    BadPcirSignature {
        /// Image offset.
        at: usize,
    },
    /// A PCIR revision other than 0 or 3.
    UnsupportedPcirRevision {
        /// Image offset.
        at: usize,
        /// The revision found.
        revision: u8,
    },
    /// A PCIR length shorter than revision 0's.
    ShortPcir {
        /// Image offset.
        at: usize,
        /// The length found.
        len: u16,
    },
    /// `ImageLength` is 0 or the image runs past the end of the ROM.
    BadImageLength {
        /// Image offset.
        at: usize,
        /// `ImageLength` in blocks.
        blocks: u16,
    },
    /// A code-type-3 image whose header does not carry `EfiSignature`.
    BadEfiSignature {
        /// Image offset.
        at: usize,
    },
    /// `InitializationSize > ImageLength`, or `EfiImageHeaderOffset >= InitializationSize` —
    /// EDK2 skips such an image (`PciOptionRomSupport.c:85-87`).
    BadEfiExtent {
        /// Image offset.
        at: usize,
    },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            ParseError::Empty => write!(f, "empty ROM"),
            ParseError::BadSignature { at } => write!(f, "image at {at:#x}: no 55 AA signature"),
            ParseError::BadPcirOffset { at, pcir } => {
                write!(
                    f,
                    "image at {at:#x}: PcirOffset {pcir:#x} is 0, unaligned or out of range"
                )
            }
            ParseError::BadPcirSignature { at } => write!(f, "image at {at:#x}: no PCIR signature"),
            ParseError::UnsupportedPcirRevision { at, revision } => {
                write!(
                    f,
                    "image at {at:#x}: PCIR revision {revision} (0 or 3 accepted)"
                )
            }
            ParseError::ShortPcir { at, len } => {
                write!(f, "image at {at:#x}: PCIR length {len:#x}")
            }
            ParseError::BadImageLength { at, blocks } => {
                write!(
                    f,
                    "image at {at:#x}: ImageLength {blocks} blocks is 0 or past the ROM"
                )
            }
            ParseError::BadEfiSignature { at } => {
                write!(
                    f,
                    "image at {at:#x}: code type 3 without EfiSignature 0x0EF1"
                )
            }
            ParseError::BadEfiExtent { at } => {
                write!(
                    f,
                    "image at {at:#x}: InitializationSize or EfiImageHeaderOffset out of range"
                )
            }
        }
    }
}

pub(crate) fn le16(b: &[u8], at: usize) -> Option<u16> {
    let s = b.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([s[0], s[1]]))
}

pub(crate) fn le32(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

pub(crate) fn le64(b: &[u8], at: usize) -> Option<u64> {
    let s = b.get(at..at.checked_add(8)?)?;
    let mut a = [0u8; 8];
    a.copy_from_slice(s);
    Some(u64::from_le_bytes(a))
}

/// Decode the image header and PCIR at `at`. The image's extent is checked against `rom`.
pub fn image_at(rom: &[u8], at: usize) -> Result<Image<'_>, ParseError> {
    if le16(rom, at) != Some(ROM_SIGNATURE) {
        return Err(ParseError::BadSignature { at });
    }
    let pcir_off = le16(rom, at + OFF_PCIR_OFFSET).ok_or(ParseError::BadSignature { at })?;
    let p = at + usize::from(pcir_off);
    if pcir_off == 0 || pcir_off % 4 != 0 || rom.len() < p + usize::from(PCIR_LEN_REV0) {
        return Err(ParseError::BadPcirOffset { at, pcir: pcir_off });
    }
    if rom[p..p + 4] != PCIR_SIGNATURE {
        return Err(ParseError::BadPcirSignature { at });
    }
    // In range: `rom.len() >= p + 0x18` was checked above, and every field read below is below 0x18.
    let r16 = |o: usize| u16::from_le_bytes([rom[p + o], rom[p + o + 1]]);
    let pcir = Pcir {
        offset: pcir_off,
        vendor: r16(0x04),
        device: r16(0x06),
        device_list: r16(0x08),
        len: r16(0x0A),
        revision: rom[p + 0x0C],
        class: ClassCode {
            prog_if: rom[p + 0x0D],
            sub: rom[p + 0x0E],
            base: rom[p + 0x0F],
        },
        image_blocks: r16(0x10),
        code_revision: r16(0x12),
        code_type: rom[p + 0x14],
        indicator: rom[p + 0x15],
    };
    if pcir.revision != 0 && pcir.revision != 3 {
        return Err(ParseError::UnsupportedPcirRevision {
            at,
            revision: pcir.revision,
        });
    }
    if pcir.len < PCIR_LEN_REV0 {
        return Err(ParseError::ShortPcir { at, len: pcir.len });
    }
    let len = usize::from(pcir.image_blocks) * BLOCK;
    let bytes = match rom.get(at..at + len) {
        Some(b) if len > 0 => b,
        _ => {
            return Err(ParseError::BadImageLength {
                at,
                blocks: pcir.image_blocks,
            });
        }
    };
    if usize::from(pcir_off) + usize::from(pcir.len) > len {
        return Err(ParseError::BadPcirOffset { at, pcir: pcir_off });
    }
    let efi = if pcir.code_type == CODE_TYPE_EFI {
        if le32(bytes, OFF_EFI_SIGNATURE) != Some(EFI_SIGNATURE) {
            return Err(ParseError::BadEfiSignature { at });
        }
        // In range: `bytes` holds at least one block, and the header is 0x1A bytes.
        let h = |o: usize| u16::from_le_bytes([bytes[o], bytes[o + 1]]);
        let efi = EfiHeader {
            init_blocks: h(0x02),
            subsystem: h(OFF_EFI_SUBSYSTEM),
            machine: h(OFF_EFI_MACHINE),
            compression: Compression::from_u16(h(OFF_COMPRESSION)),
            image_header_offset: h(OFF_IMAGE_HEADER_OFFSET),
        };
        let init = usize::from(efi.init_blocks) * BLOCK;
        if init > len || usize::from(efi.image_header_offset) >= init {
            return Err(ParseError::BadEfiExtent { at });
        }
        Some(efi)
    } else {
        None
    };
    Ok(Image {
        offset: at,
        bytes,
        pcir,
        efi,
    })
}

/// Walk a ROM image by image, the way EDK2's `PciBusDxe` does: from offset 0, each image
/// `ImageLength` blocks long, until the last-image bit or the end of the ROM. The first error ends
/// the walk.
#[must_use]
pub fn images(rom: &[u8]) -> Images<'_> {
    Images {
        rom,
        at: 0,
        done: false,
    }
}

/// The iterator [`images`] returns.
#[derive(Debug, Clone)]
pub struct Images<'a> {
    rom: &'a [u8],
    at: usize,
    done: bool,
}

impl<'a> Iterator for Images<'a> {
    type Item = Result<Image<'a>, ParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        if self.at >= self.rom.len() {
            self.done = true;
            return if self.at == 0 {
                Some(Err(ParseError::Empty))
            } else {
                None
            };
        }
        match image_at(self.rom, self.at) {
            Ok(img) => {
                self.at += img.bytes.len();
                self.done = img.is_last();
                Some(Ok(img))
            }
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}

/// Parse every image of a ROM, refusing the ROM at its first malformed image.
#[cfg(feature = "alloc")]
pub fn parse(rom: &[u8]) -> Result<alloc::vec::Vec<Image<'_>>, ParseError> {
    images(rom).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One EFI image of `blocks` blocks, PCIR revision `rev` at 0x1C.
    fn efi_image(blocks: u16, rev: u8, last: bool) -> [u8; 1024] {
        let mut b = [0u8; 1024];
        b[0] = 0x55;
        b[1] = 0xAA;
        b[2..4].copy_from_slice(&blocks.to_le_bytes());
        b[4..8].copy_from_slice(&EFI_SIGNATURE.to_le_bytes());
        b[8..10].copy_from_slice(&SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER.to_le_bytes());
        b[10..12].copy_from_slice(&MACHINE_X64.to_le_bytes());
        b[0x16..0x18].copy_from_slice(&0x200u16.to_le_bytes());
        b[0x18..0x1A].copy_from_slice(&0x1Cu16.to_le_bytes());
        b[0x1C..0x20].copy_from_slice(b"PCIR");
        b[0x1C + 0x0A..0x1C + 0x0C].copy_from_slice(&0x1Cu16.to_le_bytes());
        b[0x1C + 0x0C] = rev;
        b[0x1C + 0x0F] = 3;
        b[0x1C + 0x10..0x1C + 0x12].copy_from_slice(&blocks.to_le_bytes());
        b[0x1C + 0x14] = CODE_TYPE_EFI;
        b[0x1C + 0x15] = if last { INDICATOR_LAST } else { 0 };
        b
    }

    #[test]
    fn one_efi_image_parses_with_its_payload_extent() {
        let b = efi_image(2, 3, true);
        let imgs: [_; 1] = [images(&b).next().unwrap().unwrap()];
        let img = imgs[0];
        assert_eq!(img.pcir.revision, 3);
        assert_eq!(img.pcir.class.base, 3);
        assert!(img.is_last());
        let efi = img.efi.unwrap();
        assert_eq!(efi.compression, Compression::None);
        assert_eq!(efi.subsystem, SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER);
        assert_eq!(img.efi_payload().unwrap().len(), 0x200);
        assert!(
            images(&b).nth(1).is_none(),
            "the walk stops at the last-image bit"
        );
    }

    #[test]
    fn pcir_revisions_other_than_0_and_3_are_refused_by_name() {
        for rev in [0u8, 3] {
            assert!(
                image_at(&efi_image(2, rev, true), 0).is_ok(),
                "revision {rev}"
            );
        }
        for rev in [1u8, 2, 4] {
            assert_eq!(
                image_at(&efi_image(2, rev, true), 0),
                Err(ParseError::UnsupportedPcirRevision {
                    at: 0,
                    revision: rev
                })
            );
        }
    }

    #[test]
    fn malformed_headers_are_refused_by_name() {
        let mut b = efi_image(2, 3, true);
        b[0] = 0;
        assert_eq!(image_at(&b, 0), Err(ParseError::BadSignature { at: 0 }));

        let mut b = efi_image(2, 3, true);
        b[0x18] = 0x1E; // not a multiple of 4
        assert_eq!(
            image_at(&b, 0),
            Err(ParseError::BadPcirOffset { at: 0, pcir: 0x1E })
        );

        let mut b = efi_image(2, 3, true);
        b[0x1C] = b'X';
        assert_eq!(image_at(&b, 0), Err(ParseError::BadPcirSignature { at: 0 }));

        let b = efi_image(3, 3, true); // three blocks claimed, two present
        assert_eq!(
            image_at(&b, 0),
            Err(ParseError::BadImageLength { at: 0, blocks: 3 })
        );

        let mut b = efi_image(2, 3, true);
        b[4] = 0;
        assert_eq!(image_at(&b, 0), Err(ParseError::BadEfiSignature { at: 0 }));

        let mut b = efi_image(2, 3, true);
        b[0x16..0x18].copy_from_slice(&0x400u16.to_le_bytes()); // header offset == init size
        assert_eq!(image_at(&b, 0), Err(ParseError::BadEfiExtent { at: 0 }));

        assert_eq!(images(&[]).next(), Some(Err(ParseError::Empty)));
    }

    #[test]
    fn a_missing_last_bit_ends_at_the_rom_end_not_past_it() {
        let b = efi_image(2, 3, false);
        let mut it = images(&b);
        assert!(it.next().unwrap().is_ok());
        assert!(it.next().is_none());
    }

    #[test]
    fn class_code_round_trips_through_pcir_byte_order() {
        let c = ClassCode::from_u24(0x03_02_00);
        assert_eq!((c.base, c.sub, c.prog_if), (3, 2, 0));
        assert_eq!(c.pcir_bytes(), [0, 2, 3]);
    }
}
