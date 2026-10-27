# Building from source

There is no installer and no public binary. You build two things: the Rust workspace, and a QEMU binary that has the `kf3-gpu` device linked in. This page lists the steps the repository's build scripts perform, in their order. <!-- src: README.md, scripts/bench/build_kf3.sh -->

Read [Requirements](requirements.md) first. A QEMU binary built from this tree is not yet distributable under the GPL: see [Credits and notices](credits.md). <!-- src: LICENSE exception 2 -->

## 1. Get the source

```sh
git clone https://github.com/reindertpelsma/kayfabe.git
cd kayfabe
```
<!-- src: scripts/bench/provision_box.sh (clone URL) -->

The submodules under `third_party/` are reference sources. They are not built, linked or copied, and a normal clone does not initialise them. You do not need them to build. <!-- src: third_party/README.md, THIRD_PARTY_NOTICES.md -->

## 2. Install build packages (Debian/Ubuntu)

The package policy lives in one function of `scripts/bench/provision_box.sh`, and you can print the list instead of reading the script:

```sh
bash scripts/bench/provision_box.sh --packages full    # Rust workspace build
bash scripts/bench/provision_box.sh --packages bench   # QEMU build and the guest disk tooling
```
<!-- src: scripts/bench/provision_box.sh (kf_packages) -->

At the time of writing, `full` is `build-essential pkg-config git curl python3 kmod pciutils cpio zstd psmisc busybox-static openssh-client clang lld libssl-dev` and `bench` is `ninja-build meson python3-venv python3-pip python3-tomli libglib2.0-dev libpixman-1-dev flex bison qemu-utils cloud-image-utils genisoimage qemu-system-x86 ovmf musl-tools libslirp-dev`. The script is authoritative. <!-- src: scripts/bench/provision_box.sh -->

## 3. Install the pinned Rust toolchain

The repository pins one toolchain in `rust-toolchain.toml` (channel `1.99.0`, with `rustfmt` and `clippy`) and lists two extra targets: `x86_64-unknown-linux-musl` and `x86_64-unknown-uefi`. <!-- src: rust-toolchain.toml -->

```sh
curl -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal
. "$HOME/.cargo/env"
rustup toolchain install 1.99.0 --profile minimal
rustup target add --toolchain 1.99.0 x86_64-unknown-linux-musl x86_64-unknown-uefi
```
<!-- src: scripts/bench/provision_box.sh (rustup steps; the script reads the channel from rust-toolchain.toml and adds each target in a loop) -->

(`rustup` normally adds the listed targets itself when it installs the pinned toolchain; the commands above make it explicit, as the provisioning script does.) Without the musl standard library the whole workspace fails with `error[E0463]: can't find crate for std` on a crate that has nothing to do with musl. The UEFI target is needed because the build compiles kayfabe's boot-display driver (`firmware/kf-gop`) from source; a missing target is a named build error. No compiled binary is committed to the repository. <!-- src: scripts/bench/provision_box.sh header, crates/kf-gop-image/build.rs -->

## 4. Build and test the workspace

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

The scripts fetch **QEMU 10.2.4**. Other versions are not documented. <!-- src: scripts/bench/provision_bench_tree.sh (QEMU_VER=10.2.4) -->

```sh
curl -fsSL -o qemu-10.2.4.tar.xz https://download.qemu.org/qemu-10.2.4.tar.xz
tar -xf qemu-10.2.4.tar.xz
bash scripts/bench/build_kf3.sh "$PWD/qemu-10.2.4" [<build-dir>]
```
<!-- src: scripts/bench/provision_bench_tree.sh (download URL), scripts/bench/build_kf3.sh (usage) -->

`cargo` must be on `PATH`. The build directory defaults to `qemu-build-kf3` beside the QEMU source directory. What the script does, in order: <!-- src: scripts/bench/build_kf3.sh -->

1. Runs `cargo build --release -p kf-qemu` and requires the resulting `target/release/libkf_qemu.a`. A failed cargo step stops the script, so a stale archive is never used.
2. Copies every `qemu/hw/misc/kf3/*.c`, `*.h` and `meson.build` into `<qemu>/hw/misc/kf3/`, and the archive beside them.
3. Appends `subdir('kf3')` to `hw/misc/meson.build` and a `config KF3` stanza (`bool`, `default y if TEST_DEVICES`, `depends on PCI`) to `hw/misc/Kconfig`, unless they are already there.
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

## 6. Optional builds

**The GPU gates** (`kf-gate1` to `kf-gate9`) play the guest against the real planes on a real GPU with no QEMU: `cargo build --release -p kf-harness --bins`, run through `scripts/bench/v3_gates.sh` (see [Running the QEMU device](installing.md)). <!-- src: scripts/bench/v3_gates.sh, ARCHITECTURE.md -->

**The raw client** is the test program the 30-arm suite uses, built twice: for the host (bare-metal baseline) and as a static musl binary for the guest. <!-- src: scripts/fastguest/bare_metal_suite.sh, scripts/bench/provision_bench_tree.sh -->

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

## Not documented yet

- A one-command install, distribution packages, containers or a prebuilt binary.
- Building against a QEMU other than 10.2.4, or against a distribution's QEMU packages.
- Cross-building for arm64.
- Windows-host builds.

Verified on: `cargo test -p kf-oprom` was run for this page (Rust 1.99.0, no GPU; every test binary of that crate passed). `build_kf3.sh`, `cargo build --workspace --release`, the broker build and the thin-guest build were read from their scripts, not run for this page. Every hardware result in the repository was produced with binaries from `build_kf3.sh`. <!-- src: AGENTS.md "Verification" -->
