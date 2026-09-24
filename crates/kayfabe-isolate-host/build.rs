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

/// ★★★★★ **THE SECOND IMAGE** — `THE_CONSTRAINTS.md` §w724d, `SINGLE_STORE_PLAN.md`
/// increment 4. Built for the **host (glibc) triple**, so it has a dynamic loader and can
/// `dlopen` `libcuda.so.1`.
///
/// ⊘⊘⊘ **WHY A SECOND IMAGE AND NOT A FLAG ON THE FIRST.** `[measured 2026-09-14, locally]`
/// a musl **static-pie** binary's `dlopen` returns `NULL` with `dlerror()` =
/// *"Dynamic loading not supported"* — for `libcuda.so.1`, `libc.so.6` and `libm.so.6`
/// **alike**. The refusal is **musl's**, not `libcuda`'s, and it arrives before any question
/// about CUDA is asked. ⇒ no amount of privilege ordering rescues the static build; it has to
/// be a different one.
///
/// ⚠ **It is NOT the image every other isolate uses, and that is the whole safety argument.**
/// The static image keeps its sandbox-first guarantee unchanged. Only the ONE VM-lifetime
/// scratchpad isolate runs this build, and only it is sandboxed late.
const CUDA_IMAGE_FILE: &str = "kayfabe-isolate-cuda.image";

/// The cargo feature that asks for the second image. ⊘ Off by default, so a build that does
/// not want CUDA pays neither the build time nor a glibc-linked binary in its archive — and
/// the runtime gate then refuses **by name** rather than finding an empty blob.
const CUDA_FEATURE: &str = "CARGO_FEATURE_CUDA_SCRATCHPAD";

/// The marker that tells a nested run of this script not to recurse.
const NESTED: &str = "KAYFABE_ISOLATE_NESTED_BUILD";

/// The reviewed opt-out for a cross-*check* job — see the module docs.
const STUB: &str = "KAYFABE_ISOLATE_IMAGE_STUB";

/// ★★★★ Stamp this crate with the revision the COMPILER read, not the one an operator
/// believed they were building.
///
/// ⊘ **This existed and did not reach the ladder.** `kayfabe-qemu-raw/build.rs` derives
/// exactly this and emits it as `cargo:rustc-env` — which cargo applies to **that crate
/// only**. `kayfabe-rm-ladder` lives here, so its `option_env!("KAYFABE_BUILD_REV")` was
/// `None` unless the variable happened to be exported into the build's environment by hand.
///
/// ⚠ `[measured 2026-08-10, bench `vh`]` — a `--dictated-ring` run built with a plain
/// `cargo build --release` printed **`REV_UNDER_TEST=unstamped`**. Earlier ladder traces in
/// `traces/real_ga106/` carry real shas, so the stamp there came from a **human exporting a
/// value**: an assertion about the tree, made by the same person making the claim, in the
/// place a derivation was supposed to be. That is strictly weaker than it reads — a stale
/// binary rebuilt with a freshly typed variable claims the fresh revision, which is the
/// precise failure `CLAUDE.md`'s rev-stamp trap exists to catch.
///
/// ⇒ Derived here, from `git`, with the same shape check and the same `-dirty` suffix as
/// the qemu-raw copy. `KAYFABE_GIT_SHA` still overrides, for builds with no git.
fn stamp_build_rev() {
    println!("cargo::rerun-if-env-changed=KAYFABE_GIT_SHA");
    let sha = std::env::var("KAYFABE_GIT_SHA").ok().or_else(|| {
        let out = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8(out.stdout).ok()?.trim().to_owned();
        // ★ Shape-checked, not trusted: 40 lowercase hex. A `git` that printed a warning or
        // an empty line becomes `unknown` rather than a marker that looks like provenance.
        (s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())).then_some(s)
    });
    // ★ DIRTY is a separate question from WHICH COMMIT, and the one that matters most here:
    // a bench run is normally taken on a tree with uncommitted edits, and a
    // clean-looking sha would attribute it to a commit that does not contain them.
    let dirty = std::process::Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=no"])
        .output()
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(false);
    println!(
        "cargo::rustc-env=KAYFABE_BUILD_REV={}{}",
        sha.as_deref().unwrap_or("unknown"),
        if dirty { "-dirty" } else { "" }
    );
}

fn main() {
    stamp_build_rev();
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
    // ★★★★★ **w826 — THE SAME CLASS, ONE LEVEL FURTHER OUT.** `kayfabe-cuda` embeds
    // `cuda/walk/kf_walk.ptx` with `include_bytes!`, and that file lives OUTSIDE every
    // `crates/*/src` above. `[measured w826 ct4–ct13]` a walker fix to the PTX never reached
    // the scratchpad image: it stayed at exactly 1 203 872 bytes across the pre-fix and the
    // post-fix builds, five boots "proved the fix did not work", and a diagnosis was built
    // on them. It changed only when an unrelated edit to `cudawalk.rs` re-ran this script.
    println!(
        "cargo::rerun-if-changed={}",
        root.join("cuda").join("walk").join("kf_walk.ptx").display()
    );
    println!(
        "cargo::rerun-if-changed={}",
        root.join("Cargo.lock").display()
    );
    println!(
        "cargo::rerun-if-changed={}",
        root.join("Cargo.toml").display()
    );

    // ⊘ BOTH images are written on every early return. `isolate.rs` `include_bytes!`es each
    // unconditionally, so a file that is merely absent is a COMPILE error in a crate that has
    // nothing to do with the feature — and it was, once: the nested build wrote only the
    // first and the inner cargo then failed on the second with `No such file or directory`.
    let cuda_image_early = out_dir.join(CUDA_IMAGE_FILE);
    if std::env::var_os(NESTED).is_some() {
        // The inner build. It IS the isolate; it does not embed one.
        write(&image, &[]);
        write(&cuda_image_early, &[]);
        return;
    }
    if std::env::var_os(STUB).is_some() {
        println!(
            "cargo::warning={STUB} is set: this build embeds NO isolate image and cannot \
             spawn one. Only the cross-check job may do this."
        );
        write(&image, &[]);
        write(&cuda_image_early, &[]);
        return;
    }

    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").expect("cargo sets CARGO_CFG_TARGET_ARCH");
    let triple = format!("{arch}-unknown-linux-musl");
    let stage = out_dir.join("isolate-stage");
    let cargo = std::env::var_os("CARGO").expect("cargo sets CARGO");

    // ★★★ THE SECOND IMAGE, first because its absence must still write a file: every
    // consumer `include_bytes!`es it unconditionally, and a missing file is a build error
    // rather than a runtime refusal.
    let cuda_image = out_dir.join(CUDA_IMAGE_FILE);
    if std::env::var_os(CUDA_FEATURE).is_some() {
        let cuda_triple = format!("{arch}-unknown-linux-gnu");
        let cuda_stage = out_dir.join("isolate-cuda-stage");
        let bytes = build_isolate_for(
            &root,
            &cargo,
            &cuda_triple,
            &cuda_stage,
            &["cuda-scratchpad"],
        );
        // ⊘⊘ **THE MIRROR ASSERTION.** `the_embedded_image_is_a_static_elf` asserts the
        // ordinary image has NO `PT_INTERP`. This one must HAVE one, and the check is here
        // rather than only in a test because a statically-linked second image would `dlopen`
        // nothing and the failure would surface as "CUDA is not available on this host".
        assert!(
            has_interp(&bytes),
            "the CUDA scratchpad image was built for {cuda_triple} and has NO PT_INTERP — it \
             is statically linked, so it cannot `dlopen` libcuda and the whole point of the \
             second image is gone. §w724d's ordering does not rescue a static binary: the \
             refusal is musl's (or the linker's), not CUDA's."
        );
        write(&cuda_image, &bytes);
        println!(
            "cargo::warning=embedded CUDA scratchpad image: {} bytes, {cuda_triple} (dynamic)",
            bytes.len()
        );
    } else {
        // ⊘ EMPTY, not absent. `isolate.rs` refuses an empty image by name, which is a
        // different diagnosis from "the file is missing" (a build problem) and from "the
        // library would not load" (a host problem).
        write(&cuda_image, &[]);
    }

    // ⊘ The SAME helper the CUDA image uses, with no features: one statement of the
    // environment scrub, the recursion guard and the strip setting, so the two images cannot
    // drift into differing in something nobody chose.
    let bytes = build_isolate_for(&root, &cargo, &triple, &stage, &[]);
    assert!(
        !has_interp(&bytes),
        "the isolate image has a PT_INTERP: it is DYNAMICALLY linked, and it is `exec`'d from \
         a memfd inside a mount namespace with no path to a loader. It would fail to start \
         with ENOENT naming a file the child cannot see."
    );
    write(&image, &bytes);
    println!(
        "cargo::warning=embedded isolate image: {} bytes, {triple}",
        bytes.len()
    );
}

/// Run the nested cargo for one triple and return the bytes it produced.
///
/// ⊘ Factored out when the second image landed, so both images are built by **one**
/// statement of the environment scrub, the recursion guard and the strip setting. Two copies
/// would be two builds that could drift into differing in something nobody chose.
fn build_isolate_for(
    root: &Path,
    cargo: &std::ffi::OsStr,
    triple: &str,
    stage: &Path,
    features: &[&str],
) -> Vec<u8> {
    let mut cmd = Command::new(cargo);
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
        .current_dir(root)
        .arg("build")
        .arg("--release")
        .arg("--target")
        .arg(triple)
        .arg("--target-dir")
        .arg(stage)
        .arg("-p")
        .arg("kayfabe-isolate-host")
        .arg("--bin")
        .arg("kayfabe-isolate")
        .arg("--config")
        .arg("profile.release.strip=\"symbols\"");
    if !features.is_empty() {
        cmd.arg("--features").arg(features.join(","));
    }
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("could not run the nested cargo for {triple}: {e}"));
    assert!(
        status.success(),
        "the nested build of the isolate image failed (target {triple}).\n\
         If the standard library for that triple is missing:\n\
             rustup target add {triple}"
    );
    let built = stage.join(triple).join("release").join("kayfabe-isolate");
    let bytes = std::fs::read(&built).unwrap_or_else(|e| {
        panic!("the nested build reported success but {built:?} is unreadable: {e}")
    });
    assert!(
        bytes.starts_with(b"\x7fELF"),
        "{built:?} is not an ELF image ({} bytes)",
        bytes.len()
    );
    bytes
}

/// Whether an ELF64 image carries a `PT_INTERP` — i.e. whether it needs a dynamic loader.
///
/// ⊘ The SAME predicate `isolate.rs`'s `the_embedded_image_is_a_static_elf` uses, restated
/// here because a build script cannot call into the crate it is building. ⚠ The two must
/// agree, and they are asserted in OPPOSITE directions on the two images — which is what
/// makes "one is static and one is not" a checked fact rather than a naming convention.
fn has_interp(bytes: &[u8]) -> bool {
    const PT_INTERP: u32 = 3;
    if bytes.len() < 64 || !bytes.starts_with(b"\x7fELF") || bytes[4] != 2 {
        return false;
    }
    let u16at = |o: usize| u16::from_le_bytes([bytes[o], bytes[o + 1]]);
    let u64at = |o: usize| {
        let mut b = [0u8; 8];
        b.copy_from_slice(&bytes[o..o + 8]);
        u64::from_le_bytes(b)
    };
    let phoff = usize::try_from(u64at(0x20)).unwrap_or(0);
    let phentsize = usize::from(u16at(0x36));
    let phnum = usize::from(u16at(0x38));
    if phoff == 0 || phentsize < 4 {
        return false;
    }
    (0..phnum).any(|i| {
        let o = phoff + i * phentsize;
        o + 4 <= bytes.len()
            && u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]) == PT_INTERP
    })
}

/// Every path dependency the isolate binary is built from, so a change in any of them
/// re-runs this script. A crate missing from this list is a crate whose changes would not
/// reach the embedded image — the shape of gate this repo pins rather than derives.
const DEPENDENCY_CRATES: &[&str] = &[
    "kayfabe-isolate-host",
    "kayfabe-isolate",
    // ★★★★★ **ADDED 2026-09-14, AND IT COST A BOOT TO FIND.** `kayfabe-cuda` is linked into
    // the CUDA scratchpad image, and it was not on this list — so editing it did NOT
    // invalidate the embedded image, and cargo reused a stale one.
    //
    // ⊘⊘ **THE SHAPE IS WORSE THAN A STALE BUILD, AND IT IS WHY THIS COMMENT IS LONG.** The
    // QEMU binary's own revision stamp said HEAD. The content checks the harness runs
    // (`strings | grep 'SCRATCHPAD-CUDA AT'`) said HEAD. And the isolate image INSIDE it was
    // from the previous commit, so a fixed constant was still wrong in the boot that was
    // supposed to prove it fixed. ⇒ **a stamp on the outer artifact says nothing about an
    // artifact embedded inside it**, and every instrument this tree has for "is the binary
    // the tree" was looking at the outer one.
    "kayfabe-cuda",
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
