// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★★ Build kf3's boot-display GOP driver (`firmware/kf-gop`) from source during the ordinary cargo
//! build, and hand its bytes to `src/lib.rs` (`include_bytes!` from `OUT_DIR`).
//!
//! ## Why (`docs/OWNER_RULINGS.md` §K, 2026-10-03)
//!
//! *"We aren't going to put compiled stuff in the repo right? … the efi driver is compiled when
//! building the repo."* Until that ruling the release `kf-gop.efi` was committed beside its source and
//! `kf-oprom` embedded the committed file. Now nothing compiled is in the repository: this script
//! builds the driver for **the arch kayfabe itself is built for** (`<CARGO_CFG_TARGET_ARCH>-unknown-uefi`)
//! — so an AArch64 kayfabe carries an AArch64 driver, and `kf-oprom` copies its COFF machine into the
//! ROM's `EfiMachineType`.
//!
//! ## Why this crate, and not `kf-oprom`
//!
//! `kf-oprom` is pure: `no_std`, no I/O, in `scripts/ci/dependencies.py`'s PURE set, whose rule counts
//! build edges ("Include test/build edges"). A build script that spawns cargo is a host adapter at
//! build time. And the firmware depends on `kf-oprom` (the descriptor has one definition), so a build
//! script there would run inside every build of the very driver it builds. This crate is depended on
//! by `kf-qemu` only, and by nothing guest-side.
//!
//! ## How — the nested-cargo pattern of `crates/kayfabe-isolate-host/build.rs`
//!
//! - `cargo build --release --locked --target <arch>-unknown-uefi --bin kf-gop --manifest-path
//!   firmware/kf-gop/Cargo.toml`, into a staging target directory under `OUT_DIR`, so it never
//!   contends for the outer build's lock. The firmware is its own workspace with its own profile
//!   (`opt-level = "z"`, LTO, `strip`) and its own link flags (`firmware/kf-gop/build.rs`: subsystem 11,
//!   `/Brepro`, `/DEBUG:NONE` — the same bytes on every host, so one future signature covers all).
//! - `--locked`: `firmware/kf-gop/Cargo.lock` is the firmware's own, and a build never rewrites a file
//!   in the source tree.
//! - [`support::NESTED`] marks the inner run: the recursion guard.
//! - The inner environment is scrubbed of everything cargo exported ([`support::inherited`]);
//!   `RUSTUP_TOOLCHAIN` is kept, so the toolchain that builds the workspace builds the driver.
//! - ★ A toolchain without the UEFI target is a **named build error** before the nested build starts
//!   ([`support::missing_target_message`]); `rust-toolchain.toml` lists `x86_64-unknown-uefi`.
//! - The result is refused by name unless `kf_oprom::pe::check` accepts it and its machine is the
//!   arch's — the check `pack` applies at realize, moved to the earliest moment it can fail.
//!
//! ⊘ **No opt-out.** The isolate has `KAYFABE_ISOLATE_IMAGE_STUB` because its musl target needs a
//! linker a cross-check job lacks. The UEFI targets link with `rust-lld`, which ships with every
//! toolchain, so even a cross-check builds the real driver — CI's `aarch64` job does exactly that, for
//! `aarch64-unknown-uefi`.

#[path = "build/support.rs"]
mod support;

use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the driver's bytes land; `src/lib.rs` `include_bytes!`es it.
const IMAGE_FILE: &str = "kf-gop.efi";

/// Inputs of the nested build, relative to the workspace root. Listed explicitly because emitting
/// any `rerun-if-changed` replaces cargo's default ("any file in this package"), and none of these is
/// in this package: a missing entry is a stale driver embedded after its source changed — the class
/// `crates/kayfabe-isolate-host/build.rs` was bitten by twice.
const WATCHED: &[&str] = &[
    "firmware/kf-gop/src",
    "firmware/kf-gop/Cargo.toml",
    "firmware/kf-gop/Cargo.lock",
    "firmware/kf-gop/build.rs",
    "crates/kf-oprom/src",
    "crates/kf-oprom/Cargo.toml",
    // kf-oprom inherits its version and edition from the workspace.
    "Cargo.toml",
    "rust-toolchain.toml",
];

fn main() {
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let image = out_dir.join(IMAGE_FILE);
    let root = workspace_root();
    println!("cargo::rerun-if-env-changed={}", support::NESTED);
    for p in WATCHED {
        println!("cargo::rerun-if-changed={}", root.join(p).display());
    }

    if std::env::var_os(support::NESTED).is_some() {
        // The inner build of the firmware never needs the driver it is building. An empty image is
        // refused by name by `kf_oprom::pack` (no MZ header).
        write(&image, &[]);
        println!("cargo::rustc-env=KF_GOP_IMAGE_TARGET=none (nested build)");
        return;
    }

    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").expect("cargo sets CARGO_CFG_TARGET_ARCH");
    let (triple, machine) = support::uefi_target(&arch).unwrap_or_else(|e| panic!("{e}"));
    // ★ The named error for a missing target. Asked of the compiler cargo resolved for this build —
    // the toolchain the nested build also uses, since `RUSTUP_TOOLCHAIN` is kept. If the sysroot
    // cannot be read, the nested build's own failure below still names the fix.
    if let Some(sysroot) = sysroot()
        && !support::has_target(&sysroot, triple)
    {
        let toolchain = std::env::var("RUSTUP_TOOLCHAIN").ok();
        panic!(
            "{}",
            support::missing_target_message(triple, &sysroot, toolchain.as_deref())
        );
    }

    let cargo = std::env::var_os("CARGO").expect("cargo sets CARGO");
    let stage = out_dir.join("kf-gop-stage");
    let mut cmd = Command::new(cargo);
    for (k, _) in std::env::vars_os() {
        if !support::inherited(&k.to_string_lossy()) {
            cmd.env_remove(k);
        }
    }
    cmd.env(support::NESTED, "1")
        .current_dir(&root)
        .args(["build", "--release", "--locked", "--target", triple])
        .args(["--bin", "kf-gop", "--manifest-path"])
        .arg(root.join("firmware").join("kf-gop").join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&stage);
    let status = cmd.status().unwrap_or_else(|e| {
        panic!("kf-gop-image: could not run the nested cargo for {triple}: {e}")
    });
    assert!(
        status.success(),
        "kf-gop-image: the nested build of firmware/kf-gop for {triple} failed (its output is above).\n\
         If the standard library for {triple} is missing:\n    rustup target add {triple}"
    );
    let built = stage.join(triple).join("release").join(IMAGE_FILE);
    let bytes = std::fs::read(&built).unwrap_or_else(|e| {
        panic!("kf-gop-image: the nested build reported success but {built:?} is unreadable: {e}")
    });
    let info = kf_oprom::pe::check(&bytes)
        .unwrap_or_else(|e| panic!("kf-gop-image: {built:?} is not a driver kf3 would serve: {e}"));
    assert_eq!(
        info.machine, machine,
        "kf-gop-image: {built:?} was built for {triple} but its COFF machine is {:#06x}",
        info.machine
    );
    write(&image, &bytes);
    println!("cargo::rustc-env=KF_GOP_IMAGE_TARGET={triple}");
    println!(
        "cargo::warning=embedded boot-display GOP driver: {} bytes, {triple}",
        bytes.len()
    );
}

/// The sysroot of the compiler cargo resolved for this build, if it answers.
fn sysroot() -> Option<PathBuf> {
    let rustc = std::env::var_os("RUSTC")?;
    let out = Command::new(rustc)
        .args(["--print", "sysroot"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let s = s.trim();
    (!s.is_empty()).then(|| PathBuf::from(s))
}

/// The workspace root — this package's manifest directory, two levels up.
fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"),
    );
    manifest
        .parent()
        .and_then(Path::parent)
        .expect("crates/<pkg> is two levels below the workspace root")
        .to_path_buf()
}

/// Publish the image, creating the file even when empty so `include_bytes!` resolves.
fn write(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap_or_else(|e| panic!("writing {path:?}: {e}"));
}
