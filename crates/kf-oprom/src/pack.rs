// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! Build kf3's option ROM: one EFI image, uncompressed, carrying the host's ids and class.
//!
//! | offset | content |
//! |---|---|
//! | 0x00 | `55 AA`, `InitializationSize`, `EfiSignature 0x0EF1`, `EfiSubsystem 11`, `EfiMachineType` = **the PE's own COFF `Machine`** (`0x8664` or `0xAA64`), `CompressionType 0`, reserved, `EfiImageHeaderOffset`, `PcirOffset 0x1C` |
//! | 0x1C | PCIR revision 3, length 0x1C: host vendor, device and class, `ImageLength`, code type 3, indicator `0x80` (last image) |
//! | 0x40 | the `KFGP` descriptor ([`crate::desc`]) |
//! | … | zero padding |
//! | `EfiImageHeaderOffset` | the PE, byte-identical to the shipped `.efi`, **ending exactly on the last 512-byte block** |
//!
//! ★ Why the PE ends on the block boundary rather than starting at 0x200 with padding after it. EDK2
//! hands `LoadImage` the bytes `[EfiImageHeaderOffset, InitializationSize · 512)`
//! (`PciOptionRomSupport.c:89-90`), and its Authenticode check hashes any bytes past the sections that
//! are not the certificate table (`SecurityPkg/Library/DxeImageVerificationLib/DxeImageVerificationLib.c:556-588`,
//! edk2-stable202408). Trailing padding after a signed PE would be hashed and the signature would no
//! longer verify. Putting the padding *before* the PE keeps the loaded buffer exactly the signed file.
//! An unsigned linker output is already a multiple of 512 bytes, so for it this layout is the same as
//! "PE at 0x200". The local stand-in test (`scripts/display/gop_standin.sh`) runs both layouts under
//! Secure Boot.
//!
//! ⊘ **CORRECTED 2026-10-03 (`v3-gop`, the review of `v3-gop-kf3`) — the sentence above overstates
//! it: the end-aligned layout is UNTESTED against a firmware that denies unsigned ROMs.** OVMF trusts
//! every option ROM (`PcdOptionRomImageVerificationPolicy` 0x00, `OvmfPkg/OvmfPkgX64.dsc:689`), so the
//! stand-in's Secure Boot arms load the signed, the unsigned and the tail-padded ROM alike and can only
//! *observe* (`verdict=OBSERVED`). The layout rests on reading `DxeImageVerificationLib.c` alone.
//! Pending: a firmware arm with a deny policy (an OVMF built with
//! `PcdOptionRomImageVerificationPolicy=0x04`) in which the signed end-aligned ROM must load and the
//! tail-padded one must not (`docs/design/V3_DISPLAY.md` §4.11.9, F1).
//!
//! ★ `EfiMachineType` is copied from the PE (`docs/OWNER_RULINGS.md` §K: arch-neutral, "never a
//! constant"), so the same packer serves an x86-64 and an AArch64 build of the driver.
//!
//! ⚠ Never let QEMU patch this ROM. `pci_patch_ids` rewrites byte 6 as a checksum for a class-default
//! ROM (`hw/pci/pci.c`), and byte 6 is inside `EfiSignature` here. kf3 registers the ROM itself, in
//! the shape of `pci_add_option_rom`, and never names it as a class-level `romfile`.

use alloc::vec::Vec;
use core::fmt;

use crate::desc::{self, BootFramebuffer, DescError, Descriptor};
use crate::pe::{self, PeError};
use crate::rom::{
    BLOCK, CODE_TYPE_EFI, ClassCode, EFI_SIGNATURE, INDICATOR_LAST, OFF_COMPRESSION,
    OFF_EFI_MACHINE, OFF_EFI_SIGNATURE, OFF_EFI_SUBSYSTEM, OFF_IMAGE_HEADER_OFFSET,
    OFF_PCIR_OFFSET, PCIR_LEN_REV3, PCIR_SIGNATURE, ROM_SIGNATURE,
    SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER,
};

/// Where PCIR goes.
pub const PCIR_OFFSET: u16 = 0x1C;
/// The largest ROM a 16-bit block count can describe.
pub const MAX_ROM: usize = 0xFFFF * BLOCK;

/// The PCI identity the ROM must carry: the host GPU's, as kf3 presents it to the guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Identity {
    /// Vendor id.
    pub vendor: u16,
    /// Device id.
    pub device: u16,
    /// Class code; the base class must be 0x03 (display).
    pub class: ClassCode,
}

/// Why [`pack`] refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackError {
    /// The PE is not a kf-gop driver build ([`pe::check`]).
    Pe(PeError),
    /// The descriptor is refused ([`Descriptor::encode`]).
    Descriptor(DescError),
    /// The class is not a display controller: the firmware would never bind, and Linux's sysfb only
    /// looks for its framebuffer on display-class devices.
    NotDisplayClass(ClassCode),
    /// The ROM would exceed [`MAX_ROM`].
    TooLarge(usize),
}

impl fmt::Display for PackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            PackError::Pe(e) => write!(f, "PE: {e}"),
            PackError::Descriptor(e) => write!(f, "descriptor: {e}"),
            PackError::NotDisplayClass(c) => write!(
                f,
                "class {:02x}{:02x}{:02x} is not a display controller",
                c.base, c.sub, c.prog_if
            ),
            PackError::TooLarge(n) => write!(f, "ROM of {n} bytes exceeds {MAX_ROM}"),
        }
    }
}

/// Pack `pe` (a kf-gop build, unmodified) into a one-image PCI expansion ROM for a device with
/// identity `id`, whose boot framebuffer is `fb` and whose monitor's EDID is `edid`.
///
/// The result's length is a multiple of 512. The caller registers it as the device's ROM BAR,
/// sized to the next power of two.
pub fn pack(
    pe_bytes: &[u8],
    id: &Identity,
    fb: &BootFramebuffer,
    edid: &[u8],
) -> Result<Vec<u8>, PackError> {
    let info = pe::check(pe_bytes).map_err(PackError::Pe)?;
    if id.class.base != ClassCode::DISPLAY_BASE {
        return Err(PackError::NotDisplayClass(id.class));
    }
    let d = Descriptor {
        vendor: id.vendor,
        device: id.device,
        fb: *fb,
        edid,
    };
    let mut dbuf = [0u8; desc::MAX_LEN];
    let dlen = d.encode(&mut dbuf).map_err(PackError::Descriptor)?;
    let desc_at = desc::descriptor_offset(PCIR_OFFSET, PCIR_LEN_REV3);
    let desc_end = desc_at + dlen;
    let total = (desc_end + pe_bytes.len()).div_ceil(BLOCK) * BLOCK;
    if total > MAX_ROM {
        return Err(PackError::TooLarge(total));
    }
    let pe_at = total - pe_bytes.len();
    let blocks = (total / BLOCK) as u16; // ≤ 0xFFFF: checked against MAX_ROM above
    let mut rom = alloc::vec![0u8; total];

    rom[0..2].copy_from_slice(&ROM_SIGNATURE.to_le_bytes());
    rom[2..4].copy_from_slice(&blocks.to_le_bytes());
    rom[OFF_EFI_SIGNATURE..OFF_EFI_SIGNATURE + 4].copy_from_slice(&EFI_SIGNATURE.to_le_bytes());
    rom[OFF_EFI_SUBSYSTEM..OFF_EFI_SUBSYSTEM + 2]
        .copy_from_slice(&SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER.to_le_bytes());
    // ★ The PE's own machine (`pe::check` accepted it: x86-64 or AArch64), never a constant.
    rom[OFF_EFI_MACHINE..OFF_EFI_MACHINE + 2].copy_from_slice(&info.machine.to_le_bytes());
    rom[OFF_COMPRESSION..OFF_COMPRESSION + 2].copy_from_slice(&0u16.to_le_bytes());
    // pe_at < total ≤ MAX_ROM, but the field is 16 bits: pe_at ≤ desc_end + 511 < 0x400.
    rom[OFF_IMAGE_HEADER_OFFSET..OFF_IMAGE_HEADER_OFFSET + 2]
        .copy_from_slice(&(pe_at as u16).to_le_bytes());
    rom[OFF_PCIR_OFFSET..OFF_PCIR_OFFSET + 2].copy_from_slice(&PCIR_OFFSET.to_le_bytes());

    let p = usize::from(PCIR_OFFSET);
    rom[p..p + 4].copy_from_slice(&PCIR_SIGNATURE);
    rom[p + 0x04..p + 0x06].copy_from_slice(&id.vendor.to_le_bytes());
    rom[p + 0x06..p + 0x08].copy_from_slice(&id.device.to_le_bytes());
    // 0x08: DeviceListOffset = 0 (one device id, the one above).
    rom[p + 0x0A..p + 0x0C].copy_from_slice(&PCIR_LEN_REV3.to_le_bytes());
    rom[p + 0x0C] = 3;
    rom[p + 0x0D..p + 0x10].copy_from_slice(&id.class.pcir_bytes());
    rom[p + 0x10..p + 0x12].copy_from_slice(&blocks.to_le_bytes());
    // 0x12: CodeRevision = 0.
    rom[p + 0x14] = CODE_TYPE_EFI;
    rom[p + 0x15] = INDICATOR_LAST;
    // 0x16..0x1C: MaxRuntimeImageLength, ConfigUtilityCodeHeaderOffset, DMTFCLPEntryPointOffset = 0.

    rom[desc_at..desc_end].copy_from_slice(&dbuf[..dlen]);
    rom[pe_at..].copy_from_slice(pe_bytes);
    Ok(rom)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desc::Geometry;
    use crate::rom::{self, Compression, MACHINE_AA64, MACHINE_X64, PCIR_LEN_REV3};

    const VGA: ClassCode = ClassCode {
        base: 3,
        sub: 0,
        prog_if: 0,
    };

    fn id() -> Identity {
        Identity {
            vendor: 0x10de,
            device: 0x2504,
            class: VGA,
        }
    }

    fn fb() -> BootFramebuffer {
        BootFramebuffer {
            bar: 1,
            offset: 0,
            geometry: Geometry::for_mode(1920, 1080).unwrap(),
        }
    }

    fn driver(len: usize) -> Vec<u8> {
        let mut v = pe::synthetic(MACHINE_X64, SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER, true).to_vec();
        v.resize(len, 0xCC);
        v
    }

    #[test]
    fn the_header_fields_sit_at_the_spec_offsets() {
        let rom = pack(&driver(4096), &id(), &fb(), &[0x11; 128]).unwrap();
        assert_eq!(rom.len() % BLOCK, 0);
        assert_eq!(&rom[0..2], &[0x55, 0xAA]);
        assert_eq!(
            u32::from_le_bytes(rom[0x04..0x08].try_into().unwrap()),
            0x0EF1,
            "EfiSignature at 0x04"
        );
        assert_eq!(
            u16::from_le_bytes([rom[0x08], rom[0x09]]),
            11,
            "EfiSubsystem at 0x08"
        );
        assert_eq!(
            u16::from_le_bytes([rom[0x0A], rom[0x0B]]),
            0x8664,
            "EfiMachineType at 0x0A"
        );
        assert_eq!(
            u16::from_le_bytes([rom[0x0C], rom[0x0D]]),
            0,
            "CompressionType at 0x0C"
        );
        assert_eq!(
            u16::from_le_bytes([rom[0x16], rom[0x17]]),
            0x200,
            "EfiImageHeaderOffset at 0x16"
        );
        assert_eq!(
            u16::from_le_bytes([rom[0x18], rom[0x19]]),
            0x1C,
            "PcirOffset at 0x18"
        );
        assert_eq!(&rom[0x1C..0x20], b"PCIR");
        assert_eq!(
            &rom[0x40..0x44],
            b"KFGP",
            "descriptor at (0x1C + 0x1C + 0xF) & !0xF"
        );
    }

    /// ★ §K: the ROM's `EfiMachineType` is the PE's `Machine`. Before 2026-10-03 (`v3-gop`) it was
    /// the constant 0x8664 and an AArch64 PE was refused outright.
    #[test]
    fn the_machine_type_is_the_pes_own() {
        let mut aa64 =
            pe::synthetic(MACHINE_AA64, SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER, true).to_vec();
        aa64.resize(4096, 0xCC);
        let rom = pack(&aa64, &id(), &fb(), &[]).unwrap();
        assert_eq!(
            u16::from_le_bytes([rom[OFF_EFI_MACHINE], rom[OFF_EFI_MACHINE + 1]]),
            0xAA64
        );
        let efi = rom::image_at(&rom, 0).unwrap().efi.unwrap();
        assert_eq!(
            efi.machine, MACHINE_AA64,
            "the parser reads back what pack wrote"
        );
        let rom = pack(&driver(4096), &id(), &fb(), &[]).unwrap();
        assert_eq!(
            rom::image_at(&rom, 0).unwrap().efi.unwrap().machine,
            MACHINE_X64
        );
        let ia32 = pe::synthetic(0x014C, SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER, true);
        assert_eq!(
            pack(&ia32, &id(), &fb(), &[]),
            Err(PackError::Pe(PeError::UnsupportedMachine(0x014C)))
        );
    }

    #[test]
    fn pack_then_parse_round_trips() {
        let edid = [0x22u8; 128];
        let pe_bytes = driver(4096);
        let rom = pack(&pe_bytes, &id(), &fb(), &edid).unwrap();
        let imgs = rom::parse(&rom).unwrap();
        assert_eq!(imgs.len(), 1);
        let img = imgs[0];
        assert!(img.is_last());
        assert_eq!(img.pcir.revision, 3);
        assert_eq!(img.pcir.len, PCIR_LEN_REV3);
        assert_eq!((img.pcir.vendor, img.pcir.device), (0x10de, 0x2504));
        assert_eq!(img.pcir.class, VGA);
        assert_eq!(img.pcir.code_type, CODE_TYPE_EFI);
        assert_eq!(usize::from(img.pcir.image_blocks) * BLOCK, rom.len());
        let efi = img.efi.unwrap();
        assert_eq!(efi.compression, Compression::None);
        assert_eq!(usize::from(efi.init_blocks) * BLOCK, rom.len());
        assert_eq!(
            img.efi_payload().unwrap(),
            &pe_bytes[..],
            "LoadImage gets exactly the PE"
        );
        let d = Descriptor::find(&rom).unwrap();
        assert_eq!(d.fb, fb());
        assert_eq!(d.edid, &edid[..]);
        assert_eq!((d.vendor, d.device), (0x10de, 0x2504));
    }

    #[test]
    fn a_pe_that_is_not_block_sized_ends_on_the_boundary() {
        // A signed PE: sections (a multiple of 512) plus an 8-byte-aligned certificate table.
        let pe_bytes = driver(4096 + 0x5A8);
        let rom = pack(&pe_bytes, &id(), &fb(), &[]).unwrap();
        let efi = rom::image_at(&rom, 0).unwrap().efi.unwrap();
        let at = usize::from(efi.image_header_offset);
        assert_eq!(at + pe_bytes.len(), rom.len(), "no byte after the PE");
        assert!(
            at > 0x40 + desc::HEADER_LEN,
            "the PE starts after the descriptor"
        );
        assert_eq!(&rom[at..], &pe_bytes[..]);
        assert!(Descriptor::find(&rom).is_ok());
    }

    #[test]
    fn a_large_edid_still_leaves_room_before_the_pe() {
        let rom = pack(&driver(1024), &id(), &fb(), &[0x33; desc::EDID_MAX]).unwrap();
        let d = Descriptor::find(&rom).unwrap();
        assert_eq!(d.edid.len(), desc::EDID_MAX);
    }

    #[test]
    fn refusals_are_by_name() {
        let app = pe::synthetic(MACHINE_X64, rom::SUBSYSTEM_EFI_APPLICATION, true);
        assert_eq!(
            pack(&app, &id(), &fb(), &[]),
            Err(PackError::Pe(PeError::WrongSubsystem(10)))
        );
        let not_display = Identity {
            class: ClassCode {
                base: 2,
                sub: 0,
                prog_if: 0,
            },
            ..id()
        };
        assert_eq!(
            pack(&driver(512), &not_display, &fb(), &[]),
            Err(PackError::NotDisplayClass(not_display.class))
        );
        let bad = BootFramebuffer { bar: 7, ..fb() };
        assert_eq!(
            pack(&driver(512), &id(), &bad, &[]),
            Err(PackError::Descriptor(DescError::BadBar(7)))
        );
        assert_eq!(
            pack(&driver(MAX_ROM), &id(), &fb(), &[]),
            Err(PackError::TooLarge(MAX_ROM + BLOCK))
        );
    }

    #[test]
    fn a_3d_controller_class_is_still_a_display() {
        let id3d = Identity {
            class: ClassCode {
                base: 3,
                sub: 2,
                prog_if: 0,
            },
            ..id()
        };
        let rom = pack(&driver(512), &id3d, &fb(), &[]).unwrap();
        assert_eq!(&rom[0x1C + 0x0D..0x1C + 0x10], &[0, 2, 3]);
    }
}
