# Building from source

**Assumes:** a Debian or Ubuntu x86-64 machine (the package commands are `apt`'s; other distributions are not documented) with network access for the QEMU 10.2.4 tarball and the Rust toolchain. Building and the GPU-free tests need no GPU.

There is no public binary yet, so you build it. You build two things: the Rust workspace, and a QEMU binary that has the `kf3-gpu` device linked in. This page lists the steps of the build, in their order. Once you have the binary, continue with [Install](installing.md), step 3. <!-- src: README.md, scripts/bench/build_kf3.sh -->

Read [Requirements](requirements.md) first. No public binary is published yet; whether and when one is published is the project owner's decision, not a licence limit of the source (see [Credits and notices](credits.md)). <!-- src: docs/OWNER_RULINGS.md section BC item 1, LICENSE exception 2 -->

## 1. Get the source

<!-- kf-run: clone -->
```sh
git clone https://github.com/reindertpelsma/kayfabe.git
cd kayfabe
```
<!-- src: scripts/bench/provision_box.sh (clone URL) -->

The submodules under `third_party/` are reference sources. They are not built, linked or copied, and a normal clone does not initialise them. You do not need them to build. <!-- src: third_party/README.md, THIRD_PARTY_NOTICES.md -->

## 2. Install build packages (Debian/Ubuntu)

The package policy lives in one function of `scripts/bench/provision_box.sh`, and you can print the list instead of reading the script:

<!-- kf-run: packages_list -->
```sh
bash scripts/bench/provision_box.sh --packages full    # Rust workspace build
bash scripts/bench/provision_box.sh --packages bench   # QEMU build and the guest disk tooling
```
<!-- src: scripts/bench/provision_box.sh (kf_packages) -->

`full` is the compiler, linker and Rust-workspace set; `bench` adds what the QEMU build and the guest disk tooling need. The script's output is the list; this page does not copy it, so it cannot go stale. <!-- src: scripts/bench/provision_box.sh -->

To install both lists in one step (this needs root, so it is the one step of this page that does):

<!-- kf-run: packages_install user=root timeout=3600 -->
```sh
sudo apt-get install -y $(bash scripts/bench/provision_box.sh --packages full) $(bash scripts/bench/provision_box.sh --packages bench)
```

## 3. Install the pinned Rust toolchain

The repository pins one toolchain in `rust-toolchain.toml` (channel `1.99.0`, with `rustfmt` and `clippy`) and lists two extra targets: `x86_64-unknown-linux-musl` and `x86_64-unknown-uefi`. <!-- src: rust-toolchain.toml -->

<!-- kf-run: rust_toolchain timeout=3600 -->
```sh
curl -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal
. "$HOME/.cargo/env"
rustup toolchain install 1.99.0 --profile minimal
rustup target add --toolchain 1.99.0 x86_64-unknown-linux-musl x86_64-unknown-uefi
```
<!-- src: scripts/bench/provision_box.sh (rustup steps; the script reads the channel from rust-toolchain.toml and adds each target in a loop) -->

The first line is rustup's own official installer, which you run yourself. The repository's provisioning scripts instead fetch a pinned `rustup-init` and check it against the `.sha256` that `static.rust-lang.org` publishes next to it and a pin in `scripts/ci/trust_manifest.json`. <!-- src: scripts/ci/vast_box_main.sh, scripts/ci/trust_manifest.json -->

(`rustup` normally adds the listed targets itself when it installs the pinned toolchain; the commands above make it explicit, as the provisioning script does.) Without the musl standard library the whole workspace fails with `error[E0463]: can't find crate for std` on a crate that has nothing to do with musl. The UEFI target is needed because the build compiles kayfabe's boot-display driver (`firmware/kf-gop`) from source; a missing target is a named build error. No compiled binary is committed to the repository. <!-- src: scripts/bench/provision_box.sh header, crates/kf-gop-image/build.rs -->

## 4. Build and test the workspace

<!-- kf-run: workspace_build timeout=7200 -->
```sh
cargo build --workspace --release
cargo test -p kf-oprom          # any kf-* crate; no GPU needed
```
<!-- src: README.md "Trying it"; AGENTS.md "Verification" -->

- `cargo test -p kf-<crate>` runs without a GPU for the `kf-*` crates. <!-- src: AGENTS.md -->
- `crates/kayfabe-*` is the frozen earlier tree. It is built by `--workspace` and kept building because the test harness's raw client depends on it. <!-- src: AGENTS.md "Layout" -->
- The shipped crates have one external crate in their dependency closure (`libc`). <!-- src: THIRD_PARTY_NOTICES.md section 3 -->

## 5. Build QEMU with the `kf3-gpu` device

The device is a small C QOM device (`qemu/hw/misc/kf3/`) plus a Rust static library (`crates/kf-qemu`). `build_kf3.sh` lays the first into a QEMU source tree, links the second, and builds QEMU in its own build directory.

The scripts fetch **QEMU 10.2.4**. Other versions are not documented. <!-- src: scripts/bench/qemu.pin -->

The tarball is pinned by sha256 and size in `scripts/bench/qemu.pin`. Fetch it with the helper, which downloads it, checks it against the pin and refuses (exit 1, no file left behind) on any mismatch:

<!-- kf-run: qemu_build timeout=7200 -->
```sh
python3 -I scripts/ci/qemu_source.py fetch .          # prints ./qemu-10.2.4.tar.xz
tar -xf qemu-10.2.4.tar.xz
bash scripts/bench/build_kf3.sh --no-negctl "$PWD/qemu-10.2.4"
```

To use a tarball you already have, add `--from-file <path>`; it is checked against the same pin, and a copy that does not match is refused. There is no way to turn the check off. The pin file says how the hash was obtained and that the QEMU release signature was verified once, by hand, when it was recorded; the scripts check the hash and do not run `gpg`. `build_kf3.sh` takes an already extracted tree and does not re-check it. <!-- src: scripts/ci/qemu_source.py, scripts/bench/qemu.pin, scripts/bench/build_kf3.sh (usage) -->

`--no-negctl` leaves out the `KF3_NEGCTL_*` debug switches. They are environment variables that deliberately re-create a fixed defect, so that a test on a GPU box can show its check catches it; a binary that has them compiled in reads them. The release kit below never compiles them in, and neither does a build with `--no-negctl`. Without the option, `build_kf3.sh` is the bench build and compiles them in (a bench build prints `NEGCTL feature compiled in (bench build)` when the device starts). Use `--no-negctl` for any binary you run for real work. <!-- src: scripts/bench/build_kf3.sh, crates/kf-qemu/Cargo.toml (feature negctl) -->

`cargo` must be on `PATH`. `build_kf3.sh` takes an optional second argument, the build directory. It defaults to `qemu-build-kf3` beside the QEMU source directory. What the script does, in order: <!-- src: scripts/bench/build_kf3.sh -->

1. Runs `cargo build --release -p kf-qemu` (with `--features negctl` unless you gave `--no-negctl`) and requires the resulting `target/release/libkf_qemu.a`. A failed cargo step stops the script, so a stale archive is never used.
2. Copies every `qemu/hw/misc/kf3/*.c`, `*.h` and `meson.build` into `<qemu>/hw/misc/kf3/`, and the archive beside them.
3. Appends `subdir('kf3')` to `hw/misc/meson.build` and a `config KF3` stanza (`bool`, `default y if TEST_DEVICES`, `depends on PCI`) to `hw/misc/Kconfig`, unless they are already there. QEMU's own source is not changed by anything else unless you ask for an optional patch (see [Optional QEMU patch: sleep and wake of a Windows guest](#optional-qemu-patch-sleep-and-wake-of-a-windows-guest)). <!-- src: scripts/bench/build_kf3.sh -->
4. Takes a lock on the build directory, then configures QEMU, only if the directory is new or was configured with different flags (a stale configuration is removed whole and rebuilt):
   ```
   --target-list=x86_64-softmmu --disable-docs --disable-tools --disable-guest-agent
   --disable-werror --enable-slirp --enable-vnc --enable-pixman --disable-gtk --disable-sdl
   --disable-curses --disable-libssh --disable-vde --enable-tpm --without-default-features
   --enable-kvm --enable-system
   ```
   `--enable-pixman` and `--enable-vnc` are there because the device registers a QEMU graphic console per virtual head. `--enable-tpm` is for Windows 11 guests (a software TPM 2.0); `--enable-slirp` is for the Windows harness's user networking.
5. Runs `ninja -C <build-dir> qemu-system-x86_64`.
6. Installs the binary privately **per revision** at `<build-dir's parent>/kf3-bins/<git rev>[-dirty]/qemu-system-x86_64` (atomic rename), with `pc-bios` and `qemu-bundle` linked beside it, and prints a `KF3_BUILT <path> rev=<rev> ...` line.

The per-revision directory exists because a shared binary was once replaced under a running test. The launch scripts run the binary for their own checkout's revision. <!-- src: scripts/bench/build_kf3.sh header -->

## 5a. Build with the build kit

`scripts/release/build_qemu.py` is the kit the project's release is built with. It builds the Rust archive, lays the `kf3-gpu` device into a QEMU source tree, configures and builds QEMU, and packs the result. It installs nothing, takes no lock and needs no root. It needs the packages of step 2, `gnupg`, and the toolchain of step 3. <!-- src: scripts/release/build_qemu.py (module docstring), .github/workflows/release-qemu.yml (package list) -->

```sh
python3 -I scripts/release/build_qemu.py plan                  # the inputs, the patch list and the exact configure line; builds nothing
```

```sh
python3 -I scripts/release/build_qemu.py build --out dist      # a few minutes on 6 cores
```
<!-- src: scripts/release/build_qemu.py (plan, build); a full build of QEMU 10.2.4 took about 3 minutes on a 6-core machine (reproduce run) -->

`dist/` then holds `kayfabe-qemu-<QEMU version>-<kayfabe revision>-linux-x86_64.tar.xz` and a `...-manifest.json`. Unpack the tarball anywhere: `bin/qemu-system-x86_64` finds its firmware in `share/qemu` beside it. The manifest lists the QEMU version and how its source was checked, the kayfabe revision, every file the kit adds to the QEMU tree with its sha256, the exact configure line, the versions of the compiler and libraries, and the sha256 of each output. The configure line has no GTK and no SDL (the display is the broker, see [Display and the broker](display.md)), one target (`x86_64-softmmu`) and no network access. <!-- src: scripts/release/build_qemu.py (build, CONFIGURE_FLAGS) -->

**Which QEMU.** With no option the kit downloads the pinned official release (10.2.4) from `download.qemu.org` and accepts it only if its detached signature verifies against the release signer's key listed in `scripts/release/qemu-release.pin` (checked with a private `gpg` keyring that holds only that key; the fingerprint is the one `https://www.qemu.org/download/` publishes). To use your own QEMU source instead: <!-- src: scripts/release/qemu-release.pin, scripts/release/build_qemu.py (verify_signature) -->

```sh
python3 -I scripts/release/build_qemu.py build --qemu-tarball ~/src/qemu-10.2.4.tar.xz --out dist   # a release tarball (checked against its .sig when that file is beside it)
python3 -I scripts/release/build_qemu.py build --qemu-tree ~/src/qemu-10.2.4 --out dist             # a source tree, for example the one a distribution builds from
```

A tarball or tree you supply is yours: the manifest says so and records its hash. The kit copies a tree and never changes it, and ignores its `.git`. It accepts QEMU 10.2.4 only and refuses any other version by name, listing the versions it knows. A tree that already has a `hw/misc/kf3` directory with other files in it is refused. A tree that already contains the two one-line hunks (`subdir('kf3')` in `hw/misc/meson.build` and a `config KF3` stanza in `hw/misc/Kconfig`) gets them once. Nothing else in QEMU's source is changed unless you ask for an optional patch. <!-- src: scripts/release/build_qemu.py (QemuInput, apply_overlay, check_supported) -->

### Optional QEMU patch: sleep and wake of a Windows guest

Normally you want a stock QEMU plus the device and nothing more, and that is what every build above gives you. There is one optional patch, `hyperv-wake`, a plain unified diff in `tools/qemu-patches/hyperv-wake.patch` (its header explains the problem and lists the upstream links). It is applied only when you name it, with a command-line option (no environment variable): in the build kit, or, for a bench build, with a separate script that works on a copy of the QEMU tree. `build_kf3.sh` itself never applies it. <!-- src: tools/qemu-patches/hyperv-wake.patch (header), scripts/release/build_qemu.py (OPTIONAL_PATCHES, --with-patch), scripts/bench/build_kf3_patched.py -->

```sh
python3 -I scripts/release/build_qemu.py build --with-patch hyperv-wake --out dist
python3 -I scripts/bench/build_kf3_patched.py --with-patch hyperv-wake "$PWD/qemu-10.2.4"
```

The second command copies the tree to `patched-hyperv-wake/qemu-10.2.4` beside it, patches the copy, and runs `build_kf3.sh` on the copy. So the plain tree, its build directory and its `kf3-bins/<rev>/` binary are not touched; the patched binary is `patched-hyperv-wake/kf3-bins/<rev>/qemu-system-x86_64`. The launch scripts run the plain binary: name the patched one with `QEMU_BIN`. <!-- src: scripts/bench/build_kf3_patched.py (module docstring) -->

You can also apply the file to a QEMU 10.2.4 source tree yourself with `patch -p1 -d <tree> < tools/qemu-patches/hyperv-wake.patch`. The kit changes `pc_machine_wakeup` in `hw/i386/pc.c` and nothing else, refuses a tree whose text is not QEMU 10.2.4's, and applies the patch once. A patched build says so in its manifest (`optional_patches`, and a record of kind `optional-patch` with the file's sha256), in the name of its tarball (`...-with-hyperv-wake-linux-x86_64.tar.xz`), in `qemu-system-x86_64 --version` (`kayfabe-<revision>+hyperv-wake`) and in its `README.txt`. A build without the option contains none of it. <!-- src: scripts/release/build_qemu.py (apply_overlay, build, asset_names), scripts/ci/test_release_qemu.py -->

**Who needs it.** Only a Windows guest that has the Hyper-V enlightenments (`hv-time` with `hv-synic` and `hv-stimer`, the flags of the `B` CPU set of the Windows harness) **and** goes through ACPI sleep (S3) and wake **on a host whose TSC is stable for KVM**. QEMU resets every CPU when it wakes the guest, which clears the guest's Hyper-V time settings while the guest's clock restarts; Windows keeps reading a stale clock page and every vCPU waits in `HLT` for a timer that never fires. What was measured (one Vast box, stable-TSC host, Windows 11 with the 580.88 driver, `docs/design/V3_S3_RESUME.md` section 7b, `traces/win11_s3_wake_20261123/`): with those flags the wake hung on stock QEMU (0 of 1 wakes clean) and was clean with the patch (3 of 3). On a host whose TSC KVM does not trust (a nested box in that run), stock QEMU was clean (1 of 1). <!-- src: traces/win11_s3_wake_20261123/README.md (results table R1, R1t, R3) -->

**If you do not want the patch.** Measured: with the plain CPU set of the harness (`host,-vmx`, no `hv-*` flags), stock QEMU on the same stable-TSC host woke a Windows guest once without the hang (1 of 1; only one cycle was run, so more cycles may behave differently). That is the CPU set this manual documents for a user VM. Not measured: a Windows guest without `hv-*` flags over several cycles, and any guest on a host kernel with a different KVM. A newer host kernel is not a known way out: the KVM change that looked related ("Delay firing of expired stimers", January 2026) was put on hold by its author and no merge was found (inferred from the mailing-list threads, not from a kernel tree), and a merged KVM change that clamps an overflowing timer deadline would, by reading its description, stop the busy re-arming but not deliver the timer (inferred, not run). Linux guests have no Hyper-V enlightenments and are not affected: they passed three sleep and wake cycles on stock QEMU. <!-- src: traces/win11_s3_wake_20261123/README.md (R2), tools/qemu-patches/hyperv-wake.patch (header, upstream links); docs/design/V3_S3_RESUME.md section 7b -->

**What is not claimed.** Whether a Windows guest on a stock QEMU **without a GPU driver** shows the same hang is not known: Windows disables S3 without a graphics driver, so a bare-QEMU control could not be run. The patch makes the sleep/wake of a Windows guest with the enlightenments work in the measured configuration; it is not part of any release binary. The Windows sleep and wake row of the release gate stays expected-fail until a release run measures it. <!-- src: docs/design/V3_S3_RESUME.md sections 7b-7d; docs/RELEASE_CLAIMS.md (windows-sleep-wake) -->

**Check that a build is reproducible.** `reproduce` builds twice, in two different directories, and fails unless the binary tarball and the manifest are byte-identical:

```sh
python3 -I scripts/release/build_qemu.py reproduce --out repro   # builds twice, compares
```
<!-- src: scripts/release/build_qemu.py (reproduce); run for this page on one machine: KIT_REPRODUCIBLE for QEMU 10.2.4, for two builds from the checkout and for a build from the source tarball against one from the checkout -->

The kit fixes the clock (to the commit time of the checkout), the locale and every build path, and sorts and normalises the archive. Identical bytes are expected for the same compiler, linker and libraries, which is why the manifest records them; a different toolchain can give a different binary. <!-- src: scripts/release/build_qemu.py (build_env, pack_tree, configure_args) -->

**Verify a release you downloaded.** A release of this project carries the binary tarball, the corresponding source tarball, the manifest and a `SHA256SUMS` file, and each of them has a GitHub artifact attestation made by the workflow run that built it:

```sh
gh attestation verify kayfabe-qemu-10.2.4-<revision>-linux-x86_64.tar.xz \
  --repo reindertpelsma/kayfabe --signer-workflow reindertpelsma/kayfabe/.github/workflows/release-qemu.yml --source-ref refs/heads/master --deny-self-hosted-runners
sha256sum -c SHA256SUMS
```

The three pins matter: `--repo` alone accepts an attestation made by any workflow on any branch of the repository, while `--signer-workflow`, `--source-ref` and `--deny-self-hosted-runners` accept only the release workflow, run from `master`, on a GitHub-hosted runner. Use all of them, for every file. <!-- src: .github/workflows/release-qemu.yml (draft job) -->

To rebuild the binary from the source tarball, unpack it and point the kit inside at the tarball:

```sh
tar -xf kayfabe-qemu-10.2.4-<revision>-source.tar.xz
python3 -I kayfabe-qemu-10.2.4-<revision>-source/kayfabe/scripts/release/build_qemu.py build --source-tarball kayfabe-qemu-10.2.4-<revision>-source.tar.xz --out dist
```
<!-- src: .github/workflows/release-qemu.yml, scripts/release/build_qemu.py (make_source_tarball, open_source_tarball) -->

## 6. Optional builds

**The GPU gates** (`kf-gate1` to `kf-gate9`) play the guest against the real planes on a real GPU with no QEMU: `cargo build --release -p kf-harness --bins`, run through `scripts/bench/v3_gates.sh` (see [What you can test](test-yourself.md)). <!-- src: scripts/bench/v3_gates.sh, ARCHITECTURE.md -->

**The raw client** is the test program the 30-arm suite uses, built twice: for the host (bare-metal baseline) and as a static musl binary for the guest. <!-- src: scripts/fastguest/bare_metal_suite.sh, scripts/bench/provision_bench_tree.sh -->

<!-- kf-run: raw_client timeout=3600 -->
```sh
cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder
cargo build --release --target x86_64-unknown-linux-musl --bin kayfabe-rm-ladder
```

**The thin guest** (a kernel and an initrd that insmods the driver, runs the raw client and powers off): `bash scripts/fastguest/build_fast_guest.sh [guest.qcow2] [outdir]`. It needs `busybox` and `cpio`, a guest image that is not in use, and the raw client. <!-- src: scripts/fastguest/build_fast_guest.sh -->

**The display broker** (the host program that owns the window and input; see [Display and the broker](display.md)):

```sh
cd broker && make        # prints which backends it kept
make check               # build + selftest
```
<!-- src: broker/IMPORTED.md, broker/README.md -->

Build dependencies named in `broker/Makefile`: `libwayland-dev wayland-protocols` (Wayland backend), `libxcb1-dev libxcb-dri3-dev libxcb-present-dev` (X11 backend), optionally `libxcb-render0-dev` and `libxcb-xinput-dev`. A backend is built only when its libraries are present, and the build prints which ones it kept. <!-- src: broker/Makefile -->

## How the commands on this page are tested

The commands in the blocks above that carry a `<!-- kf-run: ... -->` line are run, as written, by the project's CI GPU machines (payload stage `install_flow`, `scripts/ci/install_flow.py`, large tier only, selected with `--only install_flow`). There is no second copy of them: `scripts/ci/doc_steps.py` reads the blocks from this file and refuses a marker it does not know. The marker convention is in that script's header. Blocks without a marker (the broker build, the GPU gates, the thin guest) are not run by it.

What the test changes, and nothing else:

- **The clone URL.** `git clone https://github.com/reindertpelsma/kayfabe.git` is replaced by a clone of a local mirror that holds exactly the commit under test, because the stage must test that commit and not whatever the public repository says. The stage then checks that the clone's `HEAD` is that commit.
- **Who runs it.** Every block runs in a fresh bash with `set -euo pipefail`, as a new unprivileged user in group `kvm` with no capabilities and a home directory under a temporary prefix. The one block marked `user=root` (`packages_install`) runs as root, where `sudo` is a no-op; the result lists which steps needed root.
- **The QEMU tarball** is downloaded as the block says and checked against the pin of `scripts/bench/qemu.pin`.

<!-- src: docs/design/V3_BOX_DOWNLOADER_AND_SUPPLY_CHAIN.md section 2 (a CI-built attested binary is designed, not built; the hook is install_flow.py --attested-binary) -->

## Not documented yet

- A one-command install, distribution packages, containers or a prebuilt binary. The page for a downloadable release is created at the first release.
- Building against a QEMU other than 10.2.4, or against a distribution's QEMU packages.
- Cross-building for arm64.
- Windows-host builds.

<!-- src (basis of this page): `cargo test -p kf-oprom` was run for this page (Rust 1.99.0, no GPU; every test binary of that crate passed). `build_kf3.sh`, `cargo build --workspace --release`, the broker build and the thin-guest build were read from their scripts, not run for this page. Every hardware result in the repository was produced with binaries from `build_kf3.sh` (AGENTS.md, "Verification"). -->
