// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! The decisions `build.rs` makes, as plain functions. A build script cannot be unit-tested, so this
//! file is compiled twice: into the build script (`#[path = "build/support.rs"]`) and into the
//! library's tests (`src/lib.rs`), which run every refusal it can name.

use std::path::Path;

/// Set in the nested cargo's environment. A run of the build script that sees it emits an empty image
/// and returns: the recursion guard of `crates/kayfabe-isolate-host/build.rs`. Nothing under
/// `firmware/kf-gop` depends on this crate today, so the guard never fires; it exists so that a
/// future dependency edge is an empty image `pack` refuses by name, not an endless nested build.
pub const NESTED: &str = "KF_GOP_IMAGE_NESTED_BUILD";

/// The UEFI triple the driver is built for when kayfabe is built for `arch`
/// (`CARGO_CFG_TARGET_ARCH`), and the COFF machine the result must carry — the value `kf-oprom` then
/// copies into the ROM's `EfiMachineType` (`docs/OWNER_RULINGS.md` §K: "build.rs builds the driver
/// for the arch kayfabe itself is built for").
///
/// # Errors
/// Any arch the ROM cannot name, by name.
pub fn uefi_target(arch: &str) -> Result<(&'static str, u16), String> {
    match arch {
        "x86_64" => Ok(("x86_64-unknown-uefi", 0x8664)),
        "aarch64" => Ok(("aarch64-unknown-uefi", 0xAA64)),
        other => Err(format!(
            "kf-gop-image: kf3's boot-display GOP driver (firmware/kf-gop) is built for the arch \
             kayfabe itself is built for, and a PCI option ROM's EFI image can name only x86-64 \
             (x86_64-unknown-uefi) or AArch64 (aarch64-unknown-uefi); this build targets `{other}` \
             (docs/OWNER_RULINGS.md §K)"
        )),
    }
}

/// Whether a variable of the outer cargo's environment reaches the nested build — the rule of
/// `crates/kayfabe-isolate-host/build.rs`. Everything cargo exported to THIS build script is dropped,
/// or the nested build would inherit the outer target, profile, flags and, under `cargo clippy`,
/// `clippy-driver` as its compiler. `CARGO_HOME` and `CARGO_NET_OFFLINE` are kept, and so is
/// `RUSTUP_TOOLCHAIN` (it matches none of the prefixes): whatever toolchain builds the workspace
/// builds the driver, so a job installs the UEFI target for the toolchain it names.
#[must_use]
pub fn inherited(name: &str) -> bool {
    name == "CARGO_HOME"
        || name == "CARGO_NET_OFFLINE"
        || !(name.starts_with("CARGO")
            || name.starts_with("RUSTC")
            || name.starts_with("RUST_")
            || name == "RUSTFLAGS"
            || name == "RUSTDOCFLAGS"
            || name == "TARGET"
            || name == "HOST"
            || name == "PROFILE"
            || name == "OUT_DIR"
            || name == "NUM_JOBS"
            || name == "DEBUG"
            || name == "OPT_LEVEL")
}

/// Whether the toolchain at `sysroot` has the standard library for `triple` (rustup's layout:
/// `<sysroot>/lib/rustlib/<triple>/lib`).
#[must_use]
pub fn has_target(sysroot: &Path, triple: &str) -> bool {
    sysroot
        .join("lib")
        .join("rustlib")
        .join(triple)
        .join("lib")
        .is_dir()
}

/// ★ The named build error for a toolchain without the UEFI target — said before the nested build
/// starts, so the failure names its cause instead of surfacing as `can't find crate for core` inside
/// a nested log.
/// `toolchain` is `RUSTUP_TOOLCHAIN` when rustup chose it, so the command names that toolchain.
#[must_use]
pub fn missing_target_message(triple: &str, sysroot: &Path, toolchain: Option<&str>) -> String {
    let which = toolchain.map_or(String::new(), |t| format!(" --toolchain {t}"));
    format!(
        "kf-gop-image: the Rust standard library for {triple} is not installed for the toolchain at \
         {}. kf3's boot-display GOP driver (firmware/kf-gop) is built from source during the ordinary \
         build (docs/OWNER_RULINGS.md §K); rust-toolchain.toml lists x86_64-unknown-uefi, and rustup \
         adds it when it installs the pinned toolchain. Add it to this toolchain with:\n    \
         rustup target add {triple}{which}",
        sysroot.display()
    )
}
