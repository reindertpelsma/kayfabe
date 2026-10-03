// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! # kf-oprom — kf3's boot-display option ROM container
//!
//! kf3 shows a picture before the guest's NVIDIA driver loads by carrying its **own** PCI expansion
//! ROM: a small UEFI GOP driver (`firmware/kf-gop`) that publishes one mode whose framebuffer is a
//! range of a kf3 BAR (`docs/design/V3_DISPLAY.md` §4.11). The PE is a constant, built once. What
//! differs per host and per VM — the PCI ids and class the ROM must carry, the framebuffer geometry,
//! the monitor's EDID — lives **outside** the PE, in a `KFGP` descriptor between the PCI data
//! structure and the image. That keeps the PE signable once (§4.11.6).
//!
//! This crate is that container, and nothing else:
//! - [`desc`] — the `KFGP` v1 descriptor ([`Descriptor`], [`BootFramebuffer`], [`Geometry`]):
//!   encode, decode, and find it inside a ROM. `no_std`, no allocation: the firmware decodes it from
//!   `EFI_PCI_IO_PROTOCOL.RomImage` with exactly this code.
//! - [`rom`] — the PCI expansion ROM layout (the EFI image header, PCIR): [`rom::images`] walks a ROM
//!   image by image. It accepts PCIR revision 0 and 3 and reports compression, because the ROMs that
//!   already ship with QEMU use revision 0 and compression 1 (`tests/efi_virtio_golden.rs`).
//! - [`pe`] — the checks a PE must pass before kf3 will serve it: PE32+, x86-64, an EFI **boot
//!   service driver** (subsystem 11), and the firmware's ABI marker.
//! - [`pack`] (feature `alloc`) — builds the ROM: EFI header, PCIR revision 3 with the host's ids and
//!   class and the last-image bit, the descriptor, and the unmodified PE, sized in 512-byte blocks.
//! - [`KF_GOP_EFI`] and [`pack_kf_gop`] (feature `embedded-gop`, on by default) — the GOP driver
//!   itself. Owner, 2026-10-03: *"generate the uefi data in kayfabe and give it as blob in the rom … all
//!   is given as config data, just like cuda."* The release build of `firmware/kf-gop` is committed
//!   beside its source as `firmware/kf-gop/kf-gop.efi` and embedded here with `include_bytes!`, the way
//!   `kf-cuda` embeds its PTX. Nothing is installed per GPU and there is no `romfile=`: kf3 calls
//!   [`pack_kf_gop`] at realize with the identity it already presents and kf-disp's geometry and EDID.
//!   CI's `firmware` job rebuilds the blob from source and fails if one byte differs (the link is
//!   reproducible: `firmware/kf-gop/build.rs`).
//!
//! The `kf-oprom` binary (`src/bin/kf-oprom.rs`) wraps [`pack`] and [`rom::images`] for scripts:
//! the local stand-in test and CI's size record.
//!
//! ★ Pure by construction: no I/O, no OS vocabulary, `unsafe_code = "forbid"` from the workspace.
//! Every refusal is a named error variant — the VMM refuses a bad geometry or PE at realize, by name,
//! instead of booting a guest whose firmware will quietly skip the device.
#![no_std]

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod desc;
#[cfg(feature = "alloc")]
pub mod pack;
pub mod pe;
pub mod rom;

/// kf3's boot-display GOP driver: the committed release build of `firmware/kf-gop`, byte for byte
/// (CI rebuilds it from source and compares). A PE32+ EFI boot-service driver for x86-64 that
/// [`pe::check`] accepts; the same bytes on every host and for every device, so one future Secure Boot
/// signature covers all of them.
#[cfg(feature = "embedded-gop")]
pub static KF_GOP_EFI: &[u8] = include_bytes!("../../../firmware/kf-gop/kf-gop.efi");

/// Pack [`KF_GOP_EFI`] for one device: the ROM kf3 registers as its ROM BAR.
#[cfg(all(feature = "embedded-gop", feature = "alloc"))]
pub fn pack_kf_gop(
    id: &Identity,
    fb: &BootFramebuffer,
    edid: &[u8],
) -> Result<alloc::vec::Vec<u8>, PackError> {
    pack(KF_GOP_EFI, id, fb, edid)
}

pub use desc::{BootFramebuffer, DescError, Descriptor, Geometry, GeometryError};
#[cfg(feature = "alloc")]
pub use pack::{Identity, PackError, pack};
pub use pe::{PeError, PeInfo};
pub use rom::{ClassCode, Compression, EfiHeader, Image, ParseError, Pcir};
