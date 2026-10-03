// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! # kf-gop-image — kf3's boot-display GOP driver, built from source and embedded
//!
//! kf3's option ROM (`docs/design/V3_DISPLAY.md` §4.11) is one constant UEFI driver wrapped, per
//! device, with generated config data (owner ruling `docs/OWNER_RULINGS.md` §K: *"generate the uefi
//! data in kayfabe and give it as blob in the rom … all is given as config data, just like cuda"*).
//! This crate is the constant half:
//!
//! - [`KF_GOP_EFI`] — `firmware/kf-gop`, compiled by this crate's build script during the ordinary
//!   cargo build for `<arch>-unknown-uefi` of the arch kayfabe is built for. ⊘ No compiled binary is
//!   committed (§K, the same day: *"the efi driver is compiled when building the repo"*); until
//!   2026-10-03 the driver was a committed `firmware/kf-gop/kf-gop.efi` embedded by `kf-oprom`.
//! - [`TARGET`] — the UEFI triple it was built for.
//! - [`pack_kf_gop`] — [`KF_GOP_EFI`] wrapped for one device by `kf_oprom::pack`: the ROM header with
//!   the driver's own machine type, PCIR with the ids and class kf3 presents, the `KFGP` descriptor.
//!
//! Its one consumer is `kf-qemu` (`BootPlan::rom`). `build.rs` says why the build lives here and not
//! in the pure `kf-oprom`.

use kf_oprom::{BootFramebuffer, Identity, PackError};

/// ★ kf3's boot-display GOP driver: `firmware/kf-gop`'s `kf-gop` binary, built by `build.rs` for
/// [`TARGET`]. An EFI boot-service driver that `kf_oprom::pe::check` accepted at build time, whose
/// COFF machine is the arch kayfabe is built for. The same bytes on every host for a given source
/// and toolchain (`firmware/kf-gop/build.rs`: `/Brepro`, `/DEBUG:NONE`), so one future Secure Boot
/// signature covers all of them.
pub static KF_GOP_EFI: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/kf-gop.efi"));

/// The UEFI triple [`KF_GOP_EFI`] was built for (`x86_64-unknown-uefi` or `aarch64-unknown-uefi`).
pub const TARGET: &str = env!("KF_GOP_IMAGE_TARGET");

/// Pack [`KF_GOP_EFI`] for one device: the ROM kf3 registers as its ROM BAR.
///
/// # Errors
/// `kf_oprom::PackError` by name — above all a class that is not a display controller.
pub fn pack_kf_gop(id: &Identity, fb: &BootFramebuffer, edid: &[u8]) -> Result<Vec<u8>, PackError> {
    kf_oprom::pack(KF_GOP_EFI, id, fb, edid)
}

/// The build script's decisions, run by the tests below (`build/support.rs`).
#[cfg(test)]
#[path = "../build/support.rs"]
mod support;

#[cfg(test)]
mod tests {
    use super::support::{NESTED, has_target, inherited, missing_target_message, uefi_target};

    /// §K: the driver is built for the arch kayfabe is built for, and only an arch a ROM can name.
    #[test]
    fn the_uefi_target_follows_the_arch_and_others_are_refused_by_name() {
        assert_eq!(uefi_target("x86_64"), Ok(("x86_64-unknown-uefi", 0x8664)));
        assert_eq!(uefi_target("aarch64"), Ok(("aarch64-unknown-uefi", 0xAA64)));
        for other in ["riscv64", "x86", "arm", "loongarch64"] {
            let e = uefi_target(other).unwrap_err();
            assert!(
                e.contains(&format!("`{other}`")) && e.contains("§K"),
                "the refusal names the arch and the ruling: {e}"
            );
        }
    }

    /// The scrub of `crates/kayfabe-isolate-host/build.rs`: nothing cargo exported to the build
    /// script reaches the nested build, except the toolchain choice and cargo's home and offline mode.
    #[test]
    fn the_nested_build_keeps_the_toolchain_and_drops_the_outer_build() {
        for kept in [
            "RUSTUP_TOOLCHAIN",
            "RUSTUP_HOME",
            "CARGO_HOME",
            "CARGO_NET_OFFLINE",
            "PATH",
            "HOME",
        ] {
            assert!(inherited(kept), "{kept} must reach the nested build");
        }
        for dropped in [
            "CARGO",
            "CARGO_TARGET_DIR",
            "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_MAKEFLAGS",
            "CARGO_CFG_TARGET_ARCH",
            "CARGO_PKG_NAME",
            "CARGO_BUILD_TARGET",
            "RUSTC",
            "RUSTC_WORKSPACE_WRAPPER",
            "RUSTC_WRAPPER",
            "RUSTFLAGS",
            "RUSTDOCFLAGS",
            "RUST_BACKTRACE",
            "TARGET",
            "HOST",
            "PROFILE",
            "OUT_DIR",
            "NUM_JOBS",
            "DEBUG",
            "OPT_LEVEL",
        ] {
            assert!(
                !inherited(dropped),
                "{dropped} must not reach the nested build"
            );
        }
        assert!(
            inherited(NESTED),
            "the recursion marker is set on the command, never scrubbed by name"
        );
    }

    /// ★ A toolchain without the UEFI target is a named build error, with the command that fixes it.
    #[test]
    fn a_missing_uefi_target_is_named_with_its_fix() {
        let dir = std::env::temp_dir().join(format!("kf-gop-image-sysroot-{}", std::process::id()));
        let lib = dir.join("lib/rustlib/x86_64-unknown-uefi/lib");
        std::fs::create_dir_all(&lib).unwrap();
        assert!(has_target(&dir, "x86_64-unknown-uefi"));
        assert!(!has_target(&dir, "aarch64-unknown-uefi"));
        let msg = missing_target_message("aarch64-unknown-uefi", &dir, None);
        assert!(
            msg.ends_with("rustup target add aarch64-unknown-uefi"),
            "{msg}"
        );
        assert!(msg.contains(&dir.display().to_string()), "{msg}");
        let msg = missing_target_message("x86_64-unknown-uefi", &dir, Some("1.99.0"));
        assert!(
            msg.ends_with("rustup target add x86_64-unknown-uefi --toolchain 1.99.0"),
            "{msg}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
