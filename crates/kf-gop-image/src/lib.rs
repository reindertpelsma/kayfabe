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
//! - [`pack_kf_gop_signed`] — the same for a signed copy of [`KF_GOP_EFI`] supplied at run time
//!   (kf3's `gop-efi=`), accepted only as that driver plus an Authenticode signature. The binary
//!   `kf-gop-export` writes [`KF_GOP_EFI`] to a file for signing (`scripts/bench/windows/`).
//!
//! Its one consumer is `kf-qemu` (`BootPlan::rom`). `build.rs` says why the build lives here and not
//! in the pure `kf-oprom`.

use kf_oprom::pe::{SigError, SignedInfo};
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

/// Why a supplied signed driver was not packed ([`pack_kf_gop_signed`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignedPackError {
    /// The file is not [`KF_GOP_EFI`] plus an Authenticode signature (`kf_oprom::pe::signed_twin_of`).
    Signature(SigError),
    /// The signed driver could not be packed (`kf_oprom::pack`).
    Pack(PackError),
}

impl core::fmt::Display for SignedPackError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SignedPackError::Signature(e) => write!(f, "signed kf-gop: {e}"),
            SignedPackError::Pack(e) => write!(f, "packing the signed kf-gop: {e}"),
        }
    }
}

impl std::error::Error for SignedPackError {}

/// ★ Pack a SIGNED copy of [`KF_GOP_EFI`] for one device (2026-10-04, branch `v3-windows`,
/// `docs/OWNER_RULINGS.md` §K: the ROM's EFI driver signed with a per-install kayfabe key enrolled in
/// the VM's `db`).
///
/// The signature is supplied at run time and never built in: the private key stays out of cargo's
/// environment and `OUT_DIR`, the embedded driver stays reproducible, and a Microsoft third-party
/// signature can arrive the same way later. `signed` is accepted only as the embedded driver plus an
/// Authenticode certificate table (`kf_oprom::pe::signed_twin_of`), so kf3 still serves exactly the
/// driver it embeds. The ROM layout ends the PE on the last 512-byte block (`kf_oprom::pack`), so the
/// signature verifies over exactly the bytes the firmware loads.
///
/// # Errors
/// [`SignedPackError`], by name.
pub fn pack_kf_gop_signed(
    signed: &[u8],
    id: &Identity,
    fb: &BootFramebuffer,
    edid: &[u8],
) -> Result<(Vec<u8>, SignedInfo), SignedPackError> {
    let info =
        kf_oprom::pe::signed_twin_of(signed, KF_GOP_EFI).map_err(SignedPackError::Signature)?;
    let rom = kf_oprom::pack(signed, id, fb, edid).map_err(SignedPackError::Pack)?;
    Ok((rom, info))
}

/// The build script's decisions, run by the tests below (`build/support.rs`).
#[cfg(test)]
#[path = "../build/support.rs"]
mod support;

#[cfg(test)]
mod tests {
    use super::support::{
        NESTED, has_target, inherited, locked_version, missing_target_message, stale_lock_message,
        uefi_target,
    };

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

    /// ★ 2026-10-03 (the review of `v3-gop`): the nested build is `--locked` against the
    /// firmware's own lock, which pins kf-oprom — a path dependency whose version is the
    /// workspace's (`crates/kf-oprom/Cargo.toml`: `version.workspace = true`, as this crate's). A
    /// version bump without `cargo update --manifest-path firmware/kf-gop/Cargo.toml -p kf-oprom`
    /// fails HERE, by name, instead of as an opaque nested `--locked` error.
    #[test]
    fn the_firmware_lock_pins_kf_oprom_at_the_workspace_version() {
        let lock = include_str!("../../../firmware/kf-gop/Cargo.lock");
        let oprom = include_str!("../../kf-oprom/Cargo.toml");
        assert!(
            oprom
                .lines()
                .any(|l| l.trim() == "version.workspace = true"),
            "kf-oprom's version is the workspace's, so this crate's version is the one to compare"
        );
        let locked = locked_version(lock, "kf-oprom");
        assert_eq!(
            locked,
            Some(env!("CARGO_PKG_VERSION")),
            "{}",
            stale_lock_message(locked, env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn a_lock_is_read_per_package_and_a_stale_one_is_named_with_its_fix() {
        let lock = "version = 4\n\n[[package]]\nname = \"kf-gop\"\nversion = \"0.1.0\"\n\n\
                    [[package]]\nname = \"kf-oprom\"\nversion = \"0.1.0\"\n";
        assert_eq!(locked_version(lock, "kf-oprom"), Some("0.1.0"));
        assert_eq!(locked_version(lock, "kf-gop"), Some("0.1.0"));
        assert_eq!(locked_version(lock, "absent"), None);
        let twice = format!("{lock}\n[[package]]\nname = \"kf-oprom\"\nversion = \"0.2.0\"\n");
        assert_eq!(
            locked_version(&twice, "kf-oprom"),
            None,
            "two versions: no single one"
        );
        let msg = stale_lock_message(Some("0.1.0"), "0.2.0");
        assert!(
            msg.contains("version 0.1.0")
                && msg.contains("is 0.2.0")
                && msg.ends_with(
                    "cargo update --manifest-path firmware/kf-gop/Cargo.toml -p kf-oprom"
                ),
            "{msg}"
        );
    }
}
