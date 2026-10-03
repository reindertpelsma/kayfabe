// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! The checks a PE must pass before kf3 serves it as its GOP.
//!
//! ★ Arch-neutral (`docs/OWNER_RULINGS.md` §K, 2026-10-03): the driver is built for the arch kayfabe
//! itself is built for, so a PE is accepted for either machine the ROM can name — x86-64 (`0x8664`)
//! or AArch64 (`0xAA64`) — and [`crate::pack`] copies the PE's own COFF `Machine` into the ROM's
//! `EfiMachineType`. Any other machine is refused by name. Which of the two a kf3 build serves is
//! decided where the driver is built (`crates/kf-gop-image`), never here.
//!
//! References: Microsoft PE/COFF specification (DOS stub `e_lfanew` at 0x3C; the COFF header; the
//! PE32+ optional header with `Subsystem` at +68 and the data directories at +112, the certificate
//! table being directory 4). Nothing here executes or relocates the image: it reads a handful of
//! header fields, bounds-checked, and searches for the firmware's ABI marker.
//!
//! The ABI marker is a byte string the firmware links in (`firmware/kf-gop/src/lib.rs`,
//! `ABI_MARKER`). It says which [`crate::desc::VERSION`] the firmware decodes, so a VMM never packs
//! a descriptor its firmware would refuse — the mismatch fails at realize, by name, not at boot.

use core::fmt;

use crate::rom::{MACHINE_AA64, MACHINE_X64, SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER, le16, le32};

/// The marker the GOP firmware carries for descriptor version 1.
pub const ABI_MARKER: &[u8] = b"KFGOP-ABI=1";

const PE32PLUS_MAGIC: u16 = 0x20B;

/// The COFF machines a kf3 option ROM may carry, with the names refusals use. The ROM's
/// `EfiMachineType` is the PE's own `Machine` ([`crate::pack`]), so this list is the whole of what
/// "arch-neutral" means here (UEFI 2.x §2.1.1 lists both as `EFI_IMAGE_MACHINE_*`).
pub const SUPPORTED_MACHINES: [(u16, &str); 2] =
    [(MACHINE_X64, "x86-64"), (MACHINE_AA64, "AArch64")];

/// The name of a supported machine, `None` for any other.
#[must_use]
pub fn machine_name(machine: u16) -> Option<&'static str> {
    SUPPORTED_MACHINES
        .iter()
        .find(|(m, _)| *m == machine)
        .map(|(_, n)| *n)
}

/// What [`inspect`] read from a PE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeInfo {
    /// COFF `Machine`.
    pub machine: u16,
    /// Optional-header `Subsystem`.
    pub subsystem: u16,
    /// `SizeOfImage`: bytes the loader allocates.
    pub size_of_image: u32,
    /// The certificate table's `(file offset, size)`, if the image is signed.
    pub cert_table: Option<(u32, u32)>,
    /// Whether [`ABI_MARKER`] occurs in the file.
    pub has_abi_marker: bool,
}

/// Why a PE is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeError {
    /// No `MZ` at offset 0.
    NoDosHeader,
    /// `e_lfanew` points outside the file, or there is no `PE\0\0` there.
    NoPeHeader,
    /// The optional header is not PE32+.
    NotPe32Plus(u16),
    /// The optional header or a data directory lies outside the file.
    Truncated,
    /// `Machine` is neither x86-64 nor AArch64 ([`SUPPORTED_MACHINES`]).
    UnsupportedMachine(u16),
    /// `Subsystem` is not an EFI boot-service driver (10 would be an EFI application, which EDK2
    /// skips inside a ROM).
    WrongSubsystem(u16),
    /// The firmware's ABI marker is absent: not a kf-gop build, or one for another descriptor
    /// version.
    NoAbiMarker,
}

impl fmt::Display for PeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            PeError::NoDosHeader => write!(f, "no MZ header"),
            PeError::NoPeHeader => write!(f, "no PE signature at e_lfanew"),
            PeError::NotPe32Plus(m) => write!(f, "optional header magic {m:#x}, not PE32+"),
            PeError::Truncated => write!(f, "headers run past the end of the file"),
            PeError::UnsupportedMachine(m) => write!(
                f,
                "machine {m:#06x} is neither x86-64 ({MACHINE_X64:#06x}) nor AArch64 ({MACHINE_AA64:#06x})"
            ),
            PeError::WrongSubsystem(s) => {
                write!(
                    f,
                    "subsystem {s}, not an EFI boot-service driver ({SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER})"
                )
            }
            PeError::NoAbiMarker => {
                write!(
                    f,
                    "no {} marker",
                    core::str::from_utf8(ABI_MARKER).unwrap_or("ABI")
                )
            }
        }
    }
}

/// Read the header fields of a PE32+ file. Refuses only what it cannot read.
pub fn inspect(pe: &[u8]) -> Result<PeInfo, PeError> {
    if pe.get(0..2) != Some(b"MZ") {
        return Err(PeError::NoDosHeader);
    }
    let lfanew = le32(pe, 0x3C).ok_or(PeError::NoDosHeader)? as usize;
    if pe.get(lfanew..lfanew.checked_add(4).ok_or(PeError::NoPeHeader)?) != Some(b"PE\0\0") {
        return Err(PeError::NoPeHeader);
    }
    let coff = lfanew + 4;
    let machine = le16(pe, coff).ok_or(PeError::Truncated)?;
    let opt = coff + 20;
    let magic = le16(pe, opt).ok_or(PeError::Truncated)?;
    if magic != PE32PLUS_MAGIC {
        return Err(PeError::NotPe32Plus(magic));
    }
    let size_of_image = le32(pe, opt + 56).ok_or(PeError::Truncated)?;
    let subsystem = le16(pe, opt + 68).ok_or(PeError::Truncated)?;
    let dirs = le32(pe, opt + 108).ok_or(PeError::Truncated)?;
    let cert_table = if dirs > 4 {
        let at = le32(pe, opt + 112 + 4 * 8).ok_or(PeError::Truncated)?;
        let size = le32(pe, opt + 112 + 4 * 8 + 4).ok_or(PeError::Truncated)?;
        (size != 0).then_some((at, size))
    } else {
        None
    };
    let has_abi_marker = pe.windows(ABI_MARKER.len()).any(|w| w == ABI_MARKER);
    Ok(PeInfo {
        machine,
        subsystem,
        size_of_image,
        cert_table,
        has_abi_marker,
    })
}

/// The checks kf3 applies before serving a PE as its GOP: a machine the ROM can name
/// ([`SUPPORTED_MACHINES`]), an EFI boot-service driver, and the ABI marker for the descriptor
/// version this crate encodes.
pub fn check(pe: &[u8]) -> Result<PeInfo, PeError> {
    let info = inspect(pe)?;
    if machine_name(info.machine).is_none() {
        return Err(PeError::UnsupportedMachine(info.machine));
    }
    if info.subsystem != SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER {
        return Err(PeError::WrongSubsystem(info.subsystem));
    }
    if !info.has_abi_marker {
        return Err(PeError::NoAbiMarker);
    }
    Ok(info)
}

/// A minimal PE32+ header for tests: `MZ`, `e_lfanew = 0x40`, COFF and optional header, five data
/// directories, then [`ABI_MARKER`]. Not loadable — it has no sections. Exposed for this crate's
/// integration tests and for the VMM's own tests of its packing path.
#[doc(hidden)]
#[must_use]
pub fn synthetic(machine: u16, subsystem: u16, marker: bool) -> [u8; 512] {
    let mut b = [0u8; 512];
    b[0..2].copy_from_slice(b"MZ");
    b[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    b[0x40..0x44].copy_from_slice(b"PE\0\0");
    b[0x44..0x46].copy_from_slice(&machine.to_le_bytes());
    let opt = 0x44 + 20;
    b[opt..opt + 2].copy_from_slice(&PE32PLUS_MAGIC.to_le_bytes());
    b[opt + 56..opt + 60].copy_from_slice(&0x3000u32.to_le_bytes());
    b[opt + 68..opt + 70].copy_from_slice(&subsystem.to_le_bytes());
    b[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes());
    if marker {
        b[0x180..0x180 + ABI_MARKER.len()].copy_from_slice(ABI_MARKER);
    }
    b
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::rom::SUBSYSTEM_EFI_APPLICATION;

    #[test]
    fn a_driver_with_the_marker_passes() {
        let pe = synthetic(MACHINE_X64, SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER, true);
        let info = check(&pe).unwrap();
        assert_eq!(info.size_of_image, 0x3000);
        assert_eq!(info.cert_table, None);
    }

    /// ★ §K: an AArch64 driver is a driver kf3 serves, with its own machine reported — before
    /// 2026-10-03 (v3-gop) `check` refused every machine but x86-64.
    #[test]
    fn an_aarch64_driver_passes_with_its_own_machine() {
        let info = check(&synthetic(
            MACHINE_AA64,
            SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER,
            true,
        ))
        .unwrap();
        assert_eq!(info.machine, 0xAA64);
        assert_eq!(machine_name(info.machine), Some("AArch64"));
        assert_eq!(machine_name(MACHINE_X64), Some("x86-64"));
    }

    /// Every other machine is refused by name: IA-32, 32-bit ARM, RISC-V 64, LoongArch 64, and none.
    #[test]
    fn a_machine_the_rom_cannot_name_is_refused_by_name() {
        for m in [0x014C_u16, 0x01C4, 0x5064, 0x6264, 0] {
            let e = check(&synthetic(m, SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER, true)).unwrap_err();
            assert_eq!(e, PeError::UnsupportedMachine(m));
            let msg = std::format!("{e}");
            assert!(
                msg.contains("x86-64") && msg.contains("AArch64"),
                "the refusal names what is accepted: {msg}"
            );
        }
    }

    #[test]
    fn an_application_a_foreign_machine_and_a_missing_marker_are_refused() {
        assert_eq!(
            check(&synthetic(MACHINE_X64, SUBSYSTEM_EFI_APPLICATION, true)),
            Err(PeError::WrongSubsystem(SUBSYSTEM_EFI_APPLICATION))
        );
        assert_eq!(
            check(&synthetic(0x014C, SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER, true)),
            Err(PeError::UnsupportedMachine(0x014C))
        );
        assert_eq!(
            check(&synthetic(
                MACHINE_X64,
                SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER,
                false
            )),
            Err(PeError::NoAbiMarker)
        );
    }

    #[test]
    fn broken_headers_are_refused_not_read_past() {
        assert_eq!(inspect(b""), Err(PeError::NoDosHeader));
        let mut pe = synthetic(MACHINE_X64, SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER, true);
        pe[0x3C..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(inspect(&pe), Err(PeError::NoPeHeader));
        let pe = synthetic(MACHINE_X64, SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER, true);
        assert_eq!(inspect(&pe[..0x60]), Err(PeError::Truncated));
        let mut pe = synthetic(MACHINE_X64, SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER, true);
        pe[0x58] = 0x0B;
        pe[0x59] = 0x01;
        assert_eq!(inspect(&pe), Err(PeError::NotPe32Plus(0x10B)));
    }

    #[test]
    fn a_certificate_table_is_reported() {
        let mut pe = synthetic(MACHINE_X64, SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER, true);
        let dir = 0x44 + 20 + 112 + 32;
        pe[dir..dir + 4].copy_from_slice(&0x400u32.to_le_bytes());
        pe[dir + 4..dir + 8].copy_from_slice(&0x5A8u32.to_le_bytes());
        assert_eq!(inspect(&pe).unwrap().cert_table, Some((0x400, 0x5A8)));
    }
}
