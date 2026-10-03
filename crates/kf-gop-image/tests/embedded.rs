// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! The driver this crate embeds — built from `firmware/kf-gop` by `build.rs`, never a committed file —
//! is one kf3 serves, for the arch kayfabe is built for, and packs into a ROM whose `EfiMachineType`
//! is its own (`docs/OWNER_RULINGS.md` §K).

use kf_gop_image::{KF_GOP_EFI, TARGET, pack_kf_gop};
use kf_oprom::desc::{BootFramebuffer, Descriptor, Geometry};
use kf_oprom::rom::{self, ClassCode, Compression, SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER};
use kf_oprom::{Identity, pe};

/// The COFF machine of the arch this test binary (and so kayfabe) was built for.
fn this_arch() -> u16 {
    match std::env::consts::ARCH {
        "x86_64" => 0x8664,
        "aarch64" => 0xAA64,
        other => panic!("kf-gop-image builds for x86_64 and aarch64 only, not {other}"),
    }
}

#[test]
fn the_embedded_driver_is_a_boot_service_driver_for_this_arch() {
    let info = pe::check(KF_GOP_EFI).expect("the built kf-gop.efi passes kf3's PE check");
    assert_eq!(info.subsystem, SUBSYSTEM_EFI_BOOT_SERVICE_DRIVER);
    assert_eq!(
        info.machine,
        this_arch(),
        "built for the arch kayfabe is built for"
    );
    assert_eq!(
        TARGET,
        format!("{}-unknown-uefi", std::env::consts::ARCH),
        "the triple build.rs chose"
    );
    assert_eq!(info.cert_table, None, "the build is unsigned");
    assert_eq!(KF_GOP_EFI.len() % 512, 0, "a linker output, file-aligned");
}

#[test]
fn it_packs_for_a_1080p_display_device_with_its_own_machine_type() {
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
        efi.machine,
        this_arch(),
        "EfiMachineType is the driver's own"
    );
    assert_eq!(
        imgs[0].efi_payload().unwrap(),
        KF_GOP_EFI,
        "LoadImage gets the driver unchanged"
    );
    let d = Descriptor::find(&romb).unwrap();
    assert_eq!((d.vendor, d.device, d.fb), (0x10de, 0x2504, fb));
    assert_eq!(d.fb.geometry.fb_size, 0x7F_0000);
    assert_eq!(d.edid, &edid[..]);
}
