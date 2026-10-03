// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! Is this PCI controller ours, and where exactly is its framebuffer?
//!
//! `Supported` runs for every controller the firmware connects, so it must say no quickly and
//! without side effects. It says yes only for a display-class controller whose ROM image (the copy
//! EDK2's PCI bus driver made, `EFI_PCI_IO_PROTOCOL.RomImage`) carries a valid `KFGP` descriptor
//! naming the controller's own vendor and device. The ids come from the descriptor that kf3 authors
//! from the host GPU (`docs/design/V3_DISPLAY.md` §4.11), so the driver names no vendor itself; the
//! local stand-in packs the same driver for QEMU's stdvga ids.
//!
//! `Start` then asks the PCI bus driver for the BAR's address and length and refuses a framebuffer
//! that does not fit it — the BAR is what the bus driver assigned, not what the descriptor hoped.

use core::fmt;

use kf_oprom::desc::{BootFramebuffer, DescError, Descriptor, EDID_MAX};
use kf_oprom::rom::ClassCode;

/// What config space says about the controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// Vendor id (config 0x00).
    pub vendor: u16,
    /// Device id (config 0x02).
    pub device: u16,
    /// Base class (config 0x0B).
    pub class_base: u8,
}

impl Config {
    /// Decode config dwords 0x00 and 0x08.
    #[must_use]
    pub fn from_dwords(id: u32, class_rev: u32) -> Config {
        Config {
            vendor: id as u16,
            device: (id >> 16) as u16,
            class_base: (class_rev >> 24) as u8,
        }
    }
}

/// Why a controller is not ours, or why `Start` gave up on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Config space could not be read through PCI I/O.
    ConfigUnreadable,
    /// Not a display controller.
    NotDisplay(u8),
    /// The PCI bus driver holds no ROM image for it.
    NoRomImage,
    /// The ROM holds no valid descriptor.
    Descriptor(DescError),
    /// The descriptor names other ids than config space.
    IdMismatch {
        /// From config space.
        config: (u16, u16),
        /// From the descriptor.
        descriptor: (u16, u16),
    },
    /// The BAR's resource descriptor is not a memory range.
    BarNotMemory,
    /// The framebuffer runs past the BAR.
    BarTooSmall {
        /// The BAR's length.
        bar_len: u64,
        /// `offset + G`.
        needs: u64,
    },
    /// The PCI bus driver cannot enable memory decode for it.
    NoMemoryDecode,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Refusal::ConfigUnreadable => write!(f, "config space unreadable"),
            Refusal::NotDisplay(c) => write!(f, "base class {c:#04x} is not a display controller"),
            Refusal::NoRomImage => write!(f, "no ROM image"),
            Refusal::Descriptor(e) => write!(f, "KFGP: {e}"),
            Refusal::IdMismatch { config, descriptor } => write!(
                f,
                "ids {:04x}:{:04x} but the descriptor names {:04x}:{:04x}",
                config.0, config.1, descriptor.0, descriptor.1
            ),
            Refusal::BarNotMemory => write!(f, "framebuffer BAR is not a memory BAR"),
            Refusal::BarTooSmall { bar_len, needs } => {
                write!(
                    f,
                    "framebuffer needs {needs:#x} bytes of a {bar_len:#x}-byte BAR"
                )
            }
            Refusal::NoMemoryDecode => write!(f, "memory decode cannot be enabled"),
        }
    }
}

/// The descriptor, copied out of the ROM image so nothing borrows the bus driver's buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Boot {
    /// Framebuffer placement and geometry.
    pub fb: BootFramebuffer,
    /// EDID bytes; the first `edid_len` are valid.
    pub edid: [u8; EDID_MAX],
    /// EDID length.
    pub edid_len: usize,
}

impl Boot {
    /// The EDID.
    #[must_use]
    pub fn edid(&self) -> &[u8] {
        &self.edid[..self.edid_len]
    }
}

/// `Supported`'s decision.
pub fn decide(cfg: &Config, rom: Option<&[u8]>) -> Result<Boot, Refusal> {
    if cfg.class_base != ClassCode::DISPLAY_BASE {
        return Err(Refusal::NotDisplay(cfg.class_base));
    }
    let rom = rom.filter(|r| !r.is_empty()).ok_or(Refusal::NoRomImage)?;
    let d = Descriptor::find(rom).map_err(Refusal::Descriptor)?;
    if (d.vendor, d.device) != (cfg.vendor, cfg.device) {
        return Err(Refusal::IdMismatch {
            config: (cfg.vendor, cfg.device),
            descriptor: (d.vendor, d.device),
        });
    }
    let mut edid = [0u8; EDID_MAX];
    edid[..d.edid.len()].copy_from_slice(d.edid);
    Ok(Boot {
        fb: d.fb,
        edid,
        edid_len: d.edid.len(),
    })
}

/// Bytes of an ACPI QWORD address-space descriptor (what `GetBarAttributes` returns for a BAR).
pub const ACPI_QWORD_LEN: usize = 46;

/// Decode `GetBarAttributes`' first resource descriptor: `(base, length)` of a memory BAR.
pub fn bar_range(acpi: &[u8]) -> Result<(u64, u64), Refusal> {
    // ACPI 6.x §6.4.3.5.1: tag 0x8A, length 0x2B, resource type 0 = memory; AddrRangeMin at 14,
    // AddrLen at 38.
    let rd = |o: usize| -> Option<u64> {
        Some(u64::from_le_bytes(acpi.get(o..o + 8)?.try_into().ok()?))
    };
    if acpi.len() < ACPI_QWORD_LEN
        || acpi[0] != 0x8A
        || acpi[1] != 0x2B
        || acpi[2] != 0
        || acpi[3] != 0
    {
        return Err(Refusal::BarNotMemory);
    }
    match (rd(14), rd(38)) {
        (Some(base), Some(len)) => Ok((base, len)),
        _ => Err(Refusal::BarNotMemory),
    }
}

/// `Start`'s placement check: the framebuffer's guest-physical base, if it fits the BAR.
pub fn place(fb: &BootFramebuffer, bar_base: u64, bar_len: u64) -> Result<u64, Refusal> {
    if !fb.fits(bar_len) {
        let needs = fb.offset.saturating_add(fb.geometry.fb_size);
        return Err(Refusal::BarTooSmall { bar_len, needs });
    }
    bar_base.checked_add(fb.offset).ok_or(Refusal::BarTooSmall {
        bar_len,
        needs: u64::MAX,
    })
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use kf_oprom::desc::Geometry;
    use kf_oprom::{Identity, pack};
    use std::vec::Vec;

    fn driver() -> Vec<u8> {
        kf_oprom::pe::synthetic(0x8664, 11, true).to_vec()
    }

    fn rom(vendor: u16, device: u16) -> Vec<u8> {
        let id = Identity {
            vendor,
            device,
            class: ClassCode {
                base: 3,
                sub: 0,
                prog_if: 0,
            },
        };
        let fb = BootFramebuffer {
            bar: 0,
            offset: 0,
            geometry: Geometry::for_mode(1152, 648).unwrap(),
        };
        pack(&driver(), &id, &fb, &[9; 128]).unwrap()
    }

    const STDVGA: Config = Config {
        vendor: 0x1234,
        device: 0x1111,
        class_base: 3,
    };

    #[test]
    fn our_rom_on_our_device_is_supported() {
        let r = rom(0x1234, 0x1111);
        let b = decide(&STDVGA, Some(&r)).unwrap();
        assert_eq!(b.fb.geometry.pitch, 4608);
        assert_eq!(b.edid(), &[9; 128][..]);
    }

    #[test]
    fn everything_else_is_refused_by_name() {
        let r = rom(0x1234, 0x1111);
        let net = Config {
            class_base: 2,
            ..STDVGA
        };
        assert_eq!(decide(&net, Some(&r)), Err(Refusal::NotDisplay(2)));
        assert_eq!(decide(&STDVGA, None), Err(Refusal::NoRomImage));
        assert_eq!(decide(&STDVGA, Some(&[])), Err(Refusal::NoRomImage));
        let other = rom(0x1234, 0x2222);
        assert_eq!(
            decide(&STDVGA, Some(&other)),
            Err(Refusal::IdMismatch {
                config: (0x1234, 0x1111),
                descriptor: (0x1234, 0x2222)
            })
        );
        let mut broken = r.clone();
        broken[0x40] = b'X';
        assert_eq!(
            decide(&STDVGA, Some(&broken)),
            Err(Refusal::Descriptor(DescError::BadMagic))
        );
    }

    #[test]
    fn config_dwords_decode() {
        let c = Config::from_dwords(0x1111_1234, 0x0300_0002);
        assert_eq!(c, STDVGA);
    }

    fn qword(ty: u8, base: u64, len: u64) -> [u8; ACPI_QWORD_LEN] {
        let mut a = [0u8; ACPI_QWORD_LEN];
        a[0] = 0x8A;
        a[1] = 0x2B;
        a[3] = ty;
        a[14..22].copy_from_slice(&base.to_le_bytes());
        a[38..46].copy_from_slice(&len.to_le_bytes());
        a
    }

    #[test]
    fn the_bar_range_comes_from_a_memory_qword() {
        assert_eq!(
            bar_range(&qword(0, 0x8000_0000, 0x100_0000)),
            Ok((0x8000_0000, 0x100_0000))
        );
        assert_eq!(
            bar_range(&qword(1, 0xC000, 0x100)),
            Err(Refusal::BarNotMemory)
        );
        assert_eq!(bar_range(&[0x79, 0]), Err(Refusal::BarNotMemory));
    }

    #[test]
    fn placement_respects_the_assigned_bar() {
        let fb = BootFramebuffer {
            bar: 1,
            offset: 0,
            geometry: Geometry::for_mode(1920, 1080).unwrap(),
        };
        assert_eq!(place(&fb, 0x80_0000_0000, 0x1000_0000), Ok(0x80_0000_0000));
        assert_eq!(
            place(&fb, 0x80_0000_0000, 0x40_0000),
            Err(Refusal::BarTooSmall {
                bar_len: 0x40_0000,
                needs: 0x7F_0000
            })
        );
    }
}
