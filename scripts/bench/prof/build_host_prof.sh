#!/usr/bin/env bash
# CPU-only, private copy of the documented build_kf3.sh flow. Never boots a VM.
set -euo pipefail
kf_build_root=${1:?private revision directory}
kf_build_rev=${2:?full source revision}
kf_qemu_source=${3:?read-only QEMU source directory}
[[ $kf_build_rev =~ ^[0-9a-f]{40}$ ]]
[[ $kf_build_root == /data/codex-K1/kf3-bins/"$kf_build_rev"-prof ]]
[[ -d $kf_build_root/src && -f $kf_qemu_source/VERSION ]]
[[ ! -e $kf_build_root/qemu-system-x86_64 && ! -e $kf_build_root/broker-prof ]]
[[ ! -e $kf_build_root/qemu-build && ! -e $kf_build_root/qemu-src ]]
exec >"$kf_build_root/build.log" 2>&1
printf '%s\n' "BUILD_PID=$$" "SOURCE_REV=$kf_build_rev"
date -u
ps -o ni= -p $$
trap 'kf_build_status=$?; printf "%s\n" "$kf_build_status" >"$kf_build_root/build.exit"; exit "$kf_build_status"' EXIT
export PATH="$HOME/.cargo/bin:$PATH"
export RUSTUP_TOOLCHAIN=1.99.0
export CARGO_TARGET_DIR="$kf_build_root/rust-target"
export KF_PROF_BUILD_REV="$kf_build_rev"
export XDG_CACHE_HOME="$kf_build_root/cache"
export TMPDIR="$kf_build_root/tmp"
mkdir "$kf_build_root/tmp" "$kf_build_root/cache" "$kf_build_root/qemu-src"
rustc --version
cc --version
cat "$kf_qemu_source/VERSION"
cd "$kf_build_root/src"
cargo build --release -j2 --locked -p kf-qemu --features prof
[[ -f $CARGO_TARGET_DIR/release/libkf_qemu.a ]]
# The shared source is read only; its existing overlay/archive is excluded.
tar -C "$kf_qemu_source" --exclude='./hw/misc/kf3' --exclude='./.git' --exclude='./build' -cf - . |
    tar -C "$kf_build_root/qemu-src" -xf -
mkdir -p "$kf_build_root/qemu-src/hw/misc/kf3"
cp qemu/hw/misc/kf3/*.c qemu/hw/misc/kf3/*.h qemu/hw/misc/kf3/meson.build "$kf_build_root/qemu-src/hw/misc/kf3/"
cp "$CARGO_TARGET_DIR/release/libkf_qemu.a" "$kf_build_root/qemu-src/hw/misc/kf3/libkf_qemu.a"
[[ -f $kf_build_root/qemu-src/hw/misc/meson.build && ! -L $kf_build_root/qemu-src/hw/misc/meson.build ]]
[[ -f $kf_build_root/qemu-src/hw/misc/Kconfig && ! -L $kf_build_root/qemu-src/hw/misc/Kconfig ]]
if ! grep -q "subdir\('kf3'\)" "$kf_build_root/qemu-src/hw/misc/meson.build"; then
    printf "\nsubdir('kf3')\n" >>"$kf_build_root/qemu-src/hw/misc/meson.build"
fi
if ! grep -q '^config KF3' "$kf_build_root/qemu-src/hw/misc/Kconfig"; then
    printf '\nconfig KF3\n    bool\n    default y if TEST_DEVICES\n    depends on PCI\n' >>"$kf_build_root/qemu-src/hw/misc/Kconfig"
fi
mkdir "$kf_build_root/qemu-build"
cd "$kf_build_root/qemu-build"
"$kf_build_root/qemu-src/configure" --target-list=x86_64-softmmu --disable-docs --disable-tools --disable-guest-agent \
    --disable-werror --enable-slirp --enable-vnc --enable-pixman --disable-gtk --disable-sdl \
    --disable-curses --disable-libssh --disable-vde --enable-tpm --without-default-features \
    --enable-kvm --enable-system
ninja -j2 qemu-system-x86_64
cd "$kf_build_root/src"
make -C broker -j2 PROF=1 PROF_REV="$kf_build_rev"
# Hard links fail if the destination exists: no published binary is replaced.
ln "$kf_build_root/qemu-build/qemu-system-x86_64" "$kf_build_root/qemu-system-x86_64"
ln "$kf_build_root/src/broker/nvkvm-display-broker" "$kf_build_root/broker-prof"
for kf_data in pc-bios qemu-bundle; do
    if [[ -e $kf_build_root/qemu-build/$kf_data ]]; then
        ln -s "$kf_build_root/qemu-build/$kf_data" "$kf_build_root/$kf_data"
    fi
done
sha256sum "$kf_build_root/qemu-system-x86_64" "$kf_build_root/broker-prof"
rm -rf "$CARGO_TARGET_DIR"
printf '%s\n' BUILD_COMPLETE
date -u
