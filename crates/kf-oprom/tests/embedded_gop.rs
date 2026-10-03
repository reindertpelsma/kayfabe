// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! The embedded GOP blob (`firmware/kf-gop/kf-gop.efi`, `kf_oprom::KF_GOP_EFI`) is a driver kf3 can
//! serve, and packs into a ROM a device would accept. CI's `firmware` job separately rebuilds the
//! blob from source and requires the same bytes; this test is what the workspace job sees of it.

use kf_oprom::desc::{BootFramebuffer, Descriptor, Geometry};
use kf_oprom::rom::{self, ClassCode, Compression, SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER};
use kf_oprom::{Identity, KF_GOP_EFI, pack_kf_gop, pe};

#[test]
fn the_embedded_blob_is_a_driver_kf3_accepts() {
    let info = pe::check(KF_GOP_EFI).expect("the committed kf-gop.efi passes kf3's PE check");
    assert_eq!(info.subsystem, SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER);
    assert_eq!(info.cert_table, None, "the committed blob is unsigned");
    assert_eq!(KF_GOP_EFI.len() % 512, 0, "a linker output, file-aligned");
}

#[test]
fn it_packs_for_a_1080p_display_device() {
    let id = Identity {
        vendor: 0x10de,
        device: 0x2504,
        class: ClassCode::from_u24(0x03_00_00),
    };
    let fb = BootFramebuffer {
        bar: 1,
        offset: 0,
        geometry: Geometry::for_mode(1920, 1080).unwrap(),
    };
    let edid = [0x5Au8; 128];
    let romb = pack_kf_gop(&id, &fb, &edid).unwrap();
    let imgs = rom::parse(&romb).unwrap();
    assert_eq!(imgs.len(), 1);
    let efi = imgs[0].efi.unwrap();
    assert_eq!(efi.compression, Compression::None);
    assert_eq!(
        imgs[0].efi_payload().unwrap(),
        KF_GOP_EFI,
        "LoadImage gets the blob unchanged"
    );
    let d = Descriptor::find(&romb).unwrap();
    assert_eq!((d.vendor, d.device, d.fb), (0x10de, 0x2504, fb));
    assert_eq!(d.fb.geometry.fb_size, 0x7F_0000);
    assert_eq!(d.edid, &edid[..]);
}
