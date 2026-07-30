//! ★★★ Build the isolate as a **static, freestanding-of-the-host** binary and hand its bytes
//! to the library, so the VMM carries its own isolate instead of looking one up by name.
//!
//! ## What this replaces, and why it is a security change rather than packaging
//!
//! The isolate used to be found at runtime by `HostIsolateFactory::locate_program()`:
//! `KAYFABE_ISOLATE_BIN` if set, otherwise a sibling of `current_exe()`. Its own rustdoc
//! named the hazard while keeping a narrower version of it — *"an isolate found on `PATH` is
//! an isolate an environment variable chose, and this process hands that binary a descriptor
//! for `/dev`"*. A sibling of `current_exe()` is chosen by whoever can write that directory.
//! Embedding deletes the category: there is no name to resolve at all.
//!
//! It is also the enabling change for the sandbox. A `pivot_root`ed child has no path to
//! `/lib64/ld-linux-*.so`, so a *dynamic* isolate can never be `exec`'d from inside its own
//! sandbox — which is why `kayfabe_linux_raw::sandbox`'s docs recorded "gated on a static
//! isolate build" as the reason the sandbox ran late.
//!
//! ## ★★ The build-ordering problem, and how it is solved
//!
//! Cargo cannot build one target for a *different* triple and embed the result: artifact
//! dependencies (`bindeps`) are nightly-only, and this workspace is pinned to stable. The C
//! solved the same problem with a build-time generator that ran the stub's own Makefile and
//! emitted `nvkvm_stub_bin.h` (`C: src/qemu/nvkvm_isolate.c:9-10`, `:246-255`).
//!
//! Here it is a **nested cargo invocation**, which is the same shape:
//!
//! - it builds `--bin kayfabe-isolate` of *this* package for `<arch>-unknown-linux-musl`,
//!   into a staging target directory under `OUT_DIR` so it cannot contend for the outer
//!   build's lock;
//! - `KAYFABE_ISOLATE_NESTED_BUILD=1` marks the inner run, and this script then emits an
//!   empty image and returns — otherwise it would recurse forever;
//! - the inner run's environment is **scrubbed** of everything cargo exported (`CARGO_*`,
//!   `RUSTC*`, `RUSTFLAGS`, `RUSTDOCFLAGS`, `TARGET`, `PROFILE`, …). Leaving them makes the
//!   inner build inherit the outer profile, the outer target, and — under `cargo clippy` —
//!   `clippy-driver` as its compiler, which is how a nested build silently stops producing a
//!   binary.
//!
//! ★ `RUSTUP_TOOLCHAIN` is deliberately **kept**. It was dropped in the first draft, on the
//! argument that resolving through `rust-toolchain.toml` makes `cargo +nightly test` embed
//! the same image as `cargo test`. That is true and it is the wrong trade: it means the
//! toolchain that must have the musl standard library installed is *not the one the job
//! selected*, so a `+nightly` job would have to install a target for a toolchain it never
//! names. Keeping it makes the rule one line — **whatever toolchain builds the workspace
//! builds the isolate** — and every CI job simply declares the musl target alongside its own.
//!
//! ## The one deliberate opt-out, and what it costs
//!
//! `KAYFABE_ISOLATE_IMAGE_STUB=1` emits an **empty** image. It exists for the aarch64
//! cross-*check* job, which type-checks the workspace for a triple it has no musl standard
//! library or linker for and never runs a test. It is not a soft-fail: nothing infers it,
//! there is no fallback that reaches for it, and `the_embedded_image_is_a_static_elf` in
//! `src/isolate.rs` turns any *test* run of such a build red at its first assertion. An empty image is also refused by `ProgramImage::from_bytes` with `ENOEXEC`
//! before a single spawn, so the failure names itself.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the embedded bytes land. `src/isolate.rs` reads it with `include_bytes!`.
const IMAGE_FILE: &str = "kayfabe-isolate.image";

/// The marker that tells a nested run of this script not to recurse.
const NESTED: &str = "KAYFABE_ISOLATE_NESTED_BUILD";

/// The reviewed opt-out for a cross-*check* job — see the module docs.
const STUB: &str = "KAYFABE_ISOLATE_IMAGE_STUB";

fn main() {
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let image = out_dir.join(IMAGE_FILE);

    println!("cargo::rerun-if-env-changed={NESTED}");
    println!("cargo::rerun-if-env-changed={STUB}");
    // ★ The sources the image is built FROM. Without these the script never re-runs and a
    // stale isolate stays embedded while its source changes — the stale-artifact class this
    // repo has already been bitten by twice. Listed explicitly because emitting any
    // `rerun-if-changed` replaces cargo's default "any file in this package".
    let root = workspace_root();
    for crate_name in DEPENDENCY_CRATES {
        println!(
            "cargo::rerun-if-changed={}",
            root.join("crates").join(crate_name).join("src").display()
        );
        println!(
            "cargo::rerun-if-changed={}",
            root.join("crates")
                .join(crate_name)
                .join("Cargo.toml")
                .display()
        );
    }
    println!(
        "cargo::rerun-if-changed={}",
        root.join("Cargo.lock").display()
    );
    println!(
        "cargo::rerun-if-changed={}",
        root.join("Cargo.toml").display()
    );

    if std::env::var_os(NESTED).is_some() {
        // The inner build. It is the isolate; it does not embed one.
        write(&image, &[]);
        return;
    }
    if std::env::var_os(STUB).is_some() {
        println!(
            "cargo::warning={STUB} is set: this build embeds NO isolate image and cannot \
             spawn one. Only the cross-check job may do this."
        );
        write(&image, &[]);
        return;
    }

    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").expect("cargo sets CARGO_CFG_TARGET_ARCH");
    let triple = format!("{arch}-unknown-linux-musl");
    let stage = out_dir.join("isolate-stage");
    let cargo = std::env::var_os("CARGO").expect("cargo sets CARGO");

    let mut cmd = Command::new(cargo);
    // ★ Scrub, do not extend. Anything cargo exported describes the OUTER build.
    for (k, _) in std::env::vars_os() {
        let name = k.to_string_lossy().into_owned();
        let inherited = name == "CARGO_HOME"
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
                || name == "OPT_LEVEL");
        if !inherited {
            cmd.env_remove(k);
        }
    }
    cmd.env(NESTED, "1")
        .current_dir(&root)
        .arg("build")
        .arg("--release")
        .arg("--target")
        .arg(&triple)
        .arg("--target-dir")
        .arg(&stage)
        .arg("-p")
        .arg("kayfabe-isolate-host")
        .arg("--bin")
        .arg("kayfabe-isolate")
        // The image is `include_bytes!`-ed into every consumer, so it is worth being small;
        // and nothing debugs the isolate through its own symbols — it is debugged through the
        // protocol, from the parent.
        .arg("--config")
        .arg("profile.release.strip=\"symbols\"");

    let status = cmd.status().unwrap_or_else(|e| {
        panic!("could not run the nested cargo that builds the isolate image: {e}")
    });
    assert!(
        status.success(),
        "the nested build of the isolate image failed (target {triple}).\n\
         If the standard library for that triple is missing:\n\
             rustup target add {triple}\n\
         The isolate MUST be static: it is `exec`'d from a memfd, inside a mount namespace \
         with no path to a dynamic loader."
    );

    let built = stage.join(&triple).join("release").join("kayfabe-isolate");
    let bytes = std::fs::read(&built).unwrap_or_else(|e| {
        panic!("the nested build reported success but {built:?} is unreadable: {e}")
    });
    assert!(
        bytes.starts_with(b"\x7fELF"),
        "{built:?} is not an ELF image ({} bytes)",
        bytes.len()
    );
    write(&image, &bytes);
    println!(
        "cargo::warning=embedded isolate image: {} bytes, {triple}",
        bytes.len()
    );
}

/// Every path dependency the isolate binary is built from, so a change in any of them
/// re-runs this script. A crate missing from this list is a crate whose changes would not
/// reach the embedded image — the shape of gate this repo pins rather than derives.
const DEPENDENCY_CRATES: &[&str] = &[
    "kayfabe-isolate-host",
    "kayfabe-isolate",
    "kayfabe-linux-raw",
    "kayfabe-util",
    "kayfabe-arch",
    "kayfabe-abi",
    "kayfabe-vmm",
];

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

/// Publish the image bytes, creating the file even when empty so `include_bytes!` resolves.
fn write(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap_or_else(|e| panic!("writing {path:?}: {e}"));
}
