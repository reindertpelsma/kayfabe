// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! Link the driver binary as an EFI **boot-service driver** (PE subsystem 11).
//!
//! `x86_64-unknown-uefi` links every binary as an EFI application (subsystem 10), and EDK2 skips an
//! application inside an option ROM (`PciOptionRomSupport.c:701`, edk2-stable202408). The flag is
//! passed per binary so the test application stays an application. Done here rather than in a
//! `.cargo/config.toml` because cargo reads config files from the working directory, not from the
//! manifest's, and CI builds with `--manifest-path`. `kf-oprom pe` checks the result (subsystem 11).
fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("uefi") {
        println!("cargo::rustc-link-arg-bin=kf-gop=/subsystem:efi_boot_service_driver");
        // Reproducible output: the PE timestamp becomes a hash of the image instead of the build
        // time, so the committed `kf-gop.efi` can be checked byte for byte against a rebuild.
        println!("cargo::rustc-link-arg-bins=/Brepro");
        // No debug directory either: it would name a `.pdb` whose file name carries cargo's per-path
        // metadata hash, so the bytes would depend on where the tree was checked out.
        println!("cargo::rustc-link-arg-bins=/DEBUG:NONE");
    }
}
