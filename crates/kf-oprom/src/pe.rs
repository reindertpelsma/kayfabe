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

// ---------------------------------------------------------------------------------------------
// ★ A Secure Boot signature, supplied at run time (2026-10-04, branch `v3-windows`, OWNER_RULINGS §K)
// ---------------------------------------------------------------------------------------------

/// `WIN_CERTIFICATE.wRevision` for the current Authenticode format (PE/COFF spec, "The Attribute
/// Certificate Table": `WIN_CERT_REVISION_2_0`).
pub const WIN_CERT_REVISION_2_0: u16 = 0x0200;
/// `WIN_CERTIFICATE.wCertificateType` for a PKCS#7 `SignedData` (`WIN_CERT_TYPE_PKCS_SIGNED_DATA`),
/// the only type UEFI's image verification accepts for a PE.
pub const WIN_CERT_TYPE_PKCS_SIGNED_DATA: u16 = 0x0002;
/// `sizeof(WIN_CERTIFICATE)` before `bCertificate`: `dwLength`, `wRevision`, `wCertificateType`.
pub const WIN_CERT_HEADER: usize = 8;

/// What [`signed_twin_of`] accepted: where the signature sits in the signed file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignedInfo {
    /// The certificate table's file offset (8-aligned, at or after the unsigned image's end).
    pub cert_offset: u32,
    /// The certificate table's size; the table ends exactly at the end of the signed file.
    pub cert_size: u32,
    /// The first `WIN_CERTIFICATE`'s `dwLength`.
    pub win_cert_len: u32,
}

/// Why a signed PE is refused as the signed twin of the embedded driver. Each case by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigError {
    /// The signed file is not a readable PE.
    SignedNotPe(PeError),
    /// The reference (embedded, unsigned) file is not a readable PE.
    UnsignedNotPe(PeError),
    /// The reference already carries a certificate table, so it is not the unsigned build.
    ReferenceIsSigned,
    /// The supplied file carries no certificate table: an unsigned build was supplied.
    NotSigned,
    /// The certificate table's offset is not 8-aligned (the PE/COFF spec requires quadword alignment).
    TableMisaligned {
        /// The table's offset.
        at: u32,
    },
    /// The certificate table does not end exactly at the end of the file: bytes follow it.
    TableNotAtEnd {
        /// The table's offset.
        at: u32,
        /// The table's size.
        size: u32,
        /// The signed file's length.
        file_len: usize,
    },
    /// The certificate table starts inside the unsigned image.
    TableInsideImage {
        /// The table's offset.
        at: u32,
        /// The unsigned image's length.
        image_len: usize,
    },
    /// A byte of the signed file differs from the embedded driver outside the two header fields
    /// signing rewrites (`CheckSum`, the Security directory entry): a different or tampered PE.
    DifferentPe {
        /// The first differing file offset.
        offset: usize,
    },
    /// A non-zero byte between the end of the unsigned image and the certificate table.
    NonZeroPadding {
        /// Its file offset.
        offset: usize,
    },
    /// The first `WIN_CERTIFICATE` is not an Authenticode PKCS#7 entry that fits the table.
    BadCertificate {
        /// `dwLength`.
        length: u32,
        /// `wRevision`.
        revision: u16,
        /// `wCertificateType`.
        cert_type: u16,
    },
}

impl fmt::Display for SigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            SigError::SignedNotPe(e) => write!(f, "the signed file is not a PE32+ image: {e}"),
            SigError::UnsignedNotPe(e) => {
                write!(f, "the reference driver is not a PE32+ image: {e}")
            }
            SigError::ReferenceIsSigned => {
                write!(
                    f,
                    "the reference driver already carries a certificate table"
                )
            }
            SigError::NotSigned => write!(
                f,
                "the file carries no certificate table (an unsigned build; sign it with sbsign)"
            ),
            SigError::TableMisaligned { at } => {
                write!(f, "the certificate table at {at:#x} is not 8-byte aligned")
            }
            SigError::TableNotAtEnd { at, size, file_len } => write!(
                f,
                "the certificate table [{at:#x}, +{size:#x}) does not end at the end of the \
                 {file_len:#x}-byte file (trailing data, or a truncated table)"
            ),
            SigError::TableInsideImage { at, image_len } => write!(
                f,
                "the certificate table at {at:#x} starts inside the {image_len:#x}-byte driver image"
            ),
            SigError::DifferentPe { offset } => write!(
                f,
                "byte {offset:#x} differs from kayfabe's embedded kf-gop driver outside CheckSum and \
                 the Security directory: not a signature of THIS build (rebuild, re-export, re-sign)"
            ),
            SigError::NonZeroPadding { offset } => write!(
                f,
                "non-zero byte at {offset:#x} between the driver image and its certificate table"
            ),
            SigError::BadCertificate {
                length,
                revision,
                cert_type,
            } => write!(
                f,
                "WIN_CERTIFICATE dwLength {length:#x} wRevision {revision:#06x} wCertificateType \
                 {cert_type:#06x} is not a PKCS#7 Authenticode entry (revision \
                 {WIN_CERT_REVISION_2_0:#06x}, type {WIN_CERT_TYPE_PKCS_SIGNED_DATA:#06x}) that fits \
                 its table"
            ),
        }
    }
}

/// ★ Accept `signed` only as `unsigned` plus an Authenticode signature, and nothing else.
///
/// What signing a PE (e.g. `sbsign`) changes, and therefore all this allows to differ: the
/// optional header's `CheckSum` (+64, 4 bytes), the Security data directory entry (directory 4,
/// +112+32, 8 bytes), zero padding to an 8-byte boundary after the image, and the certificate table
/// appended at the end. Every other byte of `unsigned` must be in `signed` at the same offset.
///
/// This checks the SHAPE of the signature, never its cryptography: the firmware verifies the
/// signature against its `db` at load time. What it guarantees is that kf3 serves exactly the driver
/// it embeds, whoever signed it.
///
/// # Errors
/// A [`SigError`], by name.
pub fn signed_twin_of(signed: &[u8], unsigned: &[u8]) -> Result<SignedInfo, SigError> {
    let u = inspect(unsigned).map_err(SigError::UnsignedNotPe)?;
    if u.cert_table.is_some() {
        return Err(SigError::ReferenceIsSigned);
    }
    let s = inspect(signed).map_err(SigError::SignedNotPe)?;
    let (at, size) = s.cert_table.ok_or(SigError::NotSigned)?;
    let (at_u, size_u) = (at as usize, size as usize);
    if !at_u.is_multiple_of(8) {
        return Err(SigError::TableMisaligned { at });
    }
    if at_u.checked_add(size_u) != Some(signed.len()) {
        return Err(SigError::TableNotAtEnd {
            at,
            size,
            file_len: signed.len(),
        });
    }
    if at_u < unsigned.len() {
        return Err(SigError::TableInsideImage {
            at,
            image_len: unsigned.len(),
        });
    }
    // The two header fields signing rewrites. `inspect` succeeded on `unsigned`, so `e_lfanew` and
    // the optional header are inside it; the same offsets in `signed` are covered by the comparison.
    let opt = le32(unsigned, 0x3C).ok_or(SigError::UnsignedNotPe(PeError::NoDosHeader))? as usize
        + 4
        + 20;
    let checksum = opt + 64..opt + 68;
    let security_dir = opt + 112 + 4 * 8..opt + 112 + 5 * 8;
    if let Some(offset) = unsigned
        .iter()
        .zip(signed)
        .enumerate()
        .find(|&(i, (a, b))| a != b && !checksum.contains(&i) && !security_dir.contains(&i))
        .map(|(i, _)| i)
    {
        return Err(SigError::DifferentPe { offset });
    }
    if let Some(i) = signed[unsigned.len()..at_u].iter().position(|&b| b != 0) {
        return Err(SigError::NonZeroPadding {
            offset: unsigned.len() + i,
        });
    }
    let length = le32(signed, at_u).unwrap_or(0);
    let revision = le16(signed, at_u + 4).unwrap_or(0);
    let cert_type = le16(signed, at_u + 6).unwrap_or(0);
    if revision != WIN_CERT_REVISION_2_0
        || cert_type != WIN_CERT_TYPE_PKCS_SIGNED_DATA
        || (length as usize) <= WIN_CERT_HEADER
        || length > size
    {
        return Err(SigError::BadCertificate {
            length,
            revision,
            cert_type,
        });
    }
    Ok(SignedInfo {
        cert_offset: at,
        cert_size: size,
        win_cert_len: length,
    })
}

/// A synthetic signature in the shape `sbsign` writes, for tests: `pe` padded with zeros to 8
/// bytes, one `WIN_CERTIFICATE` of `payload` bytes (PKCS#7 revision 2.0), padded to 8 bytes, the
/// Security directory pointed at it, and `CheckSum` rewritten. Not a valid signature.
#[doc(hidden)]
#[cfg(feature = "alloc")]
#[must_use]
pub fn synthetic_signed(pe: &[u8], payload: usize) -> alloc::vec::Vec<u8> {
    let mut out = alloc::vec::Vec::from(pe);
    out.resize(pe.len().div_ceil(8) * 8, 0);
    let at = out.len();
    let len = WIN_CERT_HEADER + payload;
    out.extend_from_slice(&(len as u32).to_le_bytes());
    out.extend_from_slice(&WIN_CERT_REVISION_2_0.to_le_bytes());
    out.extend_from_slice(&WIN_CERT_TYPE_PKCS_SIGNED_DATA.to_le_bytes());
    out.extend(core::iter::repeat_n(0xA5u8, payload));
    out.resize(out.len().div_ceil(8) * 8, 0);
    let size = out.len() - at;
    let opt = le32(pe, 0x3C).unwrap_or(0) as usize + 24;
    out[opt + 112 + 32..opt + 112 + 36].copy_from_slice(&(at as u32).to_le_bytes());
    out[opt + 112 + 36..opt + 112 + 40].copy_from_slice(&(size as u32).to_le_bytes());
    out[opt + 64..opt + 68].copy_from_slice(&0x0001_2345u32.to_le_bytes());
    out
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

    fn driver() -> [u8; 512] {
        synthetic(MACHINE_X64, SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER, true)
    }

    /// ★ 2026-10-04 (v3-windows): an sbsign-shaped signature of the embedded driver is accepted,
    /// and says where its table is.
    #[test]
    fn an_sbsign_shaped_signature_of_the_same_driver_is_accepted() {
        let signed = synthetic_signed(&driver(), 1500);
        let info = signed_twin_of(&signed, &driver()).unwrap();
        assert_eq!(info.cert_offset, 512);
        assert_eq!(info.win_cert_len, 8 + 1500);
        assert_eq!(
            info.cert_offset as usize + info.cert_size as usize,
            signed.len()
        );
        assert!(
            check(&signed).is_ok(),
            "the signed file still passes the driver checks"
        );
    }

    /// An image whose length is not a multiple of 8 gets zero padding before the table.
    #[test]
    fn zero_padding_before_the_table_is_accepted_and_non_zero_padding_is_not() {
        let unsigned = &driver()[..509];
        let signed = synthetic_signed(unsigned, 64);
        assert_eq!(signed_twin_of(&signed, unsigned).unwrap().cert_offset, 512);
        let mut bad = signed.clone();
        bad[510] = 1;
        assert_eq!(
            signed_twin_of(&bad, unsigned),
            Err(SigError::NonZeroPadding { offset: 510 })
        );
    }

    #[test]
    fn a_tampered_byte_is_refused_with_its_offset() {
        let mut signed = synthetic_signed(&driver(), 64);
        signed[0x1F0] ^= 1;
        assert_eq!(
            signed_twin_of(&signed, &driver()),
            Err(SigError::DifferentPe { offset: 0x1F0 })
        );
        let msg = std::format!("{}", SigError::DifferentPe { offset: 0x1F0 });
        assert!(msg.contains("0x1f0") && msg.contains("re-sign"), "{msg}");
    }

    #[test]
    fn a_different_pe_is_refused() {
        let other = synthetic(MACHINE_AA64, SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER, true);
        let signed = synthetic_signed(&other, 64);
        assert_eq!(
            signed_twin_of(&signed, &driver()),
            Err(SigError::DifferentPe { offset: 0x45 })
        );
    }

    #[test]
    fn trailing_data_after_the_table_is_refused() {
        let mut signed = synthetic_signed(&driver(), 64);
        signed.extend_from_slice(&[0; 8]);
        assert!(matches!(
            signed_twin_of(&signed, &driver()),
            Err(SigError::TableNotAtEnd { at: 512, .. })
        ));
    }

    #[test]
    fn a_table_inside_the_image_or_misaligned_is_refused() {
        let mut signed = synthetic_signed(&driver(), 64);
        let dir = 0x44 + 20 + 112 + 32;
        let size = (signed.len() - 0x100) as u32;
        signed[dir..dir + 4].copy_from_slice(&0x100u32.to_le_bytes());
        signed[dir + 4..dir + 8].copy_from_slice(&size.to_le_bytes());
        assert_eq!(
            signed_twin_of(&signed, &driver()),
            Err(SigError::TableInsideImage {
                at: 0x100,
                image_len: 512
            })
        );
        signed[dir..dir + 4].copy_from_slice(&0x204u32.to_le_bytes());
        assert_eq!(
            signed_twin_of(&signed, &driver()),
            Err(SigError::TableMisaligned { at: 0x204 })
        );
    }

    #[test]
    fn an_unsigned_file_and_a_signed_reference_are_refused() {
        assert_eq!(
            signed_twin_of(&driver(), &driver()),
            Err(SigError::NotSigned)
        );
        let signed = synthetic_signed(&driver(), 64);
        assert_eq!(
            signed_twin_of(&signed, &signed),
            Err(SigError::ReferenceIsSigned)
        );
        assert_eq!(
            signed_twin_of(b"not a pe", &driver()),
            Err(SigError::SignedNotPe(PeError::NoDosHeader))
        );
    }

    #[test]
    fn a_certificate_that_is_not_pkcs7_or_overflows_its_table_is_refused() {
        let signed = synthetic_signed(&driver(), 64);
        let mut wrong_type = signed.clone();
        wrong_type[512 + 6] = 1; // WIN_CERT_TYPE_X509
        assert!(matches!(
            signed_twin_of(&wrong_type, &driver()),
            Err(SigError::BadCertificate { cert_type: 1, .. })
        ));
        let mut too_long = signed.clone();
        too_long[512..516].copy_from_slice(&0x1000u32.to_le_bytes());
        assert!(matches!(
            signed_twin_of(&too_long, &driver()),
            Err(SigError::BadCertificate { length: 0x1000, .. })
        ));
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
