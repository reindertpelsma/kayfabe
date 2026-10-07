// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! Golden parse of a ROM kf3 did not write: QEMU's `efi-virtio.rom` (iPXE), as Ubuntu ships it in
//! `qemu-system-data` (`/usr/share/qemu/efi-virtio.rom`, 524 288 bytes, sha256
//! `16c41f627d9d681c300230a20f4c43a2ed722d83cfcd76a5139ac9a9c99822a5`).
//!
//! The expected values were read from that file on 2026-10-03 with an independent Python decoder
//! before this crate's parser existed, and they agree with the GOP design's review note: two images;
//! the second is the EFI image, with PCIR **revision 0, length 0x18**, `CompressionType` 1 and
//! `EfiImageHeaderOffset` 0x38 — not the revision-3, uncompressed shape `pack` emits. That is why
//! `parse` accepts both revisions and reports compression.
//!
//! Two arms:
//! - `the_recorded_headers_parse_to_the_read_values` runs everywhere. It rebuilds the file's layout
//!   from the header bytes alone (the two image headers and PCIRs, copied below; every other byte
//!   zero), so CI needs no QEMU package. The fixture is factual header data, 108 bytes.
//! - `the_installed_rom_parses_to_the_same_values` parses the real file when it is installed, and
//!   first checks that the fixture's bytes are the file's bytes. Where the file is absent it says so
//!   on stderr and passes — the first arm is the one that always runs.

use kf_oprom::desc::Descriptor;
use kf_oprom::rom::{self, ClassCode, Compression, Image};

const ROM_LEN: usize = 0x8_0000;
const IMAGE1_AT: usize = 0x1_A200;

/// `efi-virtio.rom[0x0..0x38]`: the legacy image's header and its revision-3 PCIR.
const IMAGE0_HEAD: [u8; 0x38] = [
    0x55, 0xaa, 0xd1, 0xe9, 0xa2, 0x00, 0x69, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, //
    0x9c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x84, 0x00, 0x1c, 0x00, 0x40, 0x00, 0x50, 0x43, 0x49,
    0x52, //
    0xf4, 0x1a, 0x41, 0x10, 0xc0, 0x04, 0x1c, 0x00, 0x03, 0x00, 0x00, 0x02, 0xd1, 0x00, 0x01,
    0x00, //
    0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// `efi-virtio.rom[0x1A200..0x1A234]`: the EFI image's header and its revision-0 PCIR.
const IMAGE1_HEAD: [u8; 0x34] = [
    0x55, 0xaa, 0x60, 0x00, 0xf1, 0x0e, 0x00, 0x00, 0x0b, 0x00, 0x64, 0x86, 0x01, 0x00, 0x00,
    0x00, //
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x38, 0x00, 0x1c, 0x00, 0x00, 0x00, 0x50, 0x43, 0x49,
    0x52, //
    0xf4, 0x1a, 0x41, 0x10, 0x00, 0x00, 0x18, 0x00, 0x00, 0x00, 0x00, 0x02, 0x60, 0x00, 0x00,
    0x00, //
    0x03, 0x80, 0x00, 0x00,
];

fn rebuilt() -> Vec<u8> {
    let mut v = vec![0u8; ROM_LEN];
    v[..IMAGE0_HEAD.len()].copy_from_slice(&IMAGE0_HEAD);
    v[IMAGE1_AT..IMAGE1_AT + IMAGE1_HEAD.len()].copy_from_slice(&IMAGE1_HEAD);
    v
}

const NET: ClassCode = ClassCode {
    base: 0x02,
    sub: 0x00,
    prog_if: 0x00,
};

fn assert_read_values(imgs: &[Image<'_>]) {
    assert_eq!(
        imgs.len(),
        2,
        "two images, the second carrying the last-image bit"
    );

    let legacy = &imgs[0];
    assert_eq!(legacy.offset, 0);
    assert_eq!(legacy.bytes.len(), 209 * 512);
    assert_eq!(legacy.pcir.offset, 0x1C);
    assert_eq!((legacy.pcir.vendor, legacy.pcir.device), (0x1af4, 0x1041));
    assert_eq!(legacy.pcir.device_list, 0x4C0);
    assert_eq!((legacy.pcir.revision, legacy.pcir.len), (3, 0x1C));
    assert_eq!(legacy.pcir.class, NET);
    assert_eq!(legacy.pcir.image_blocks, 209);
    assert_eq!(legacy.pcir.code_revision, 1);
    assert_eq!(legacy.pcir.code_type, rom::CODE_TYPE_PCAT);
    assert_eq!(legacy.pcir.indicator, 0);
    assert_eq!(
        legacy.efi, None,
        "a legacy image's bytes after offset 2 are not EFI fields"
    );

    let efi_img = &imgs[1];
    assert_eq!(efi_img.offset, IMAGE1_AT);
    assert_eq!(efi_img.bytes.len(), 96 * 512);
    assert_eq!(efi_img.pcir.offset, 0x1C);
    assert_eq!((efi_img.pcir.vendor, efi_img.pcir.device), (0x1af4, 0x1041));
    assert_eq!(
        (efi_img.pcir.revision, efi_img.pcir.len),
        (0, 0x18),
        "PCIR revision 0, length 0x18"
    );
    assert_eq!(efi_img.pcir.class, NET);
    assert_eq!(efi_img.pcir.image_blocks, 96);
    assert_eq!(efi_img.pcir.code_type, rom::CODE_TYPE_EFI);
    assert_eq!(efi_img.pcir.indicator, 0x80);
    assert!(efi_img.is_last());
    let efi = efi_img.efi.expect("code type 3 carries the EFI header");
    assert_eq!(efi.init_blocks, 96);
    assert_eq!(efi.subsystem, rom::SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER);
    assert_eq!(efi.machine, rom::MACHINE_X64);
    assert_eq!(efi.compression, Compression::Efi, "CompressionType 1");
    assert_eq!(efi.image_header_offset, 0x38);
    assert_eq!(
        efi_img.efi_payload().map(<[u8]>::len),
        Some(96 * 512 - 0x38)
    );
}

#[test]
fn the_recorded_headers_parse_to_the_read_values() {
    let bytes = rebuilt();
    let imgs = rom::parse(&bytes).expect("efi-virtio.rom's layout parses");
    assert_read_values(&imgs);
    // Not one of ours: the first image is legacy, so there is no descriptor to find.
    assert!(Descriptor::find(&bytes).is_err());
}

#[test]
fn the_installed_rom_parses_to_the_same_values() {
    let path = "/usr/share/qemu/efi-virtio.rom";
    let Ok(bytes) = std::fs::read(path) else {
        eprintln!("SKIP {path} is not installed; the recorded-headers arm covers the parser");
        return;
    };
    assert_eq!(
        bytes.len(),
        ROM_LEN,
        "the file this golden test was recorded from"
    );
    assert_eq!(
        &bytes[..IMAGE0_HEAD.len()],
        &IMAGE0_HEAD[..],
        "fixture = file, image 0"
    );
    assert_eq!(
        &bytes[IMAGE1_AT..IMAGE1_AT + IMAGE1_HEAD.len()],
        &IMAGE1_HEAD[..],
        "fixture = file, image 1"
    );
    let imgs = rom::parse(&bytes).expect("the installed ROM parses");
    assert_read_values(&imgs);
}
