#!/usr/bin/env bash
# ★ Build QEMU with the v3 kf3-gpu device: the Rust archive (crates/kf-qemu → libkf_qemu.a) + the C
# overlay (qemu/hw/misc/kf3) laid into a QEMU source tree with two one-line hunks, built in its OWN
# build dir (the old nvkvm build is untouched).
#
#   usage: bash scripts/bench/build_kf3.sh <qemu-source-tree> [<build-dir>]
#
# ⊘ The archive is REFUSED if cargo did not just produce it (the bench once served a binary built
# from a stale revision for weeks); the result line names the revision and the binary's mtime.
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
QEMU="$(cd "${1:?usage: build_kf3.sh <qemu-source-tree> [<build-dir>]}" && pwd)"
BUILD="${2:-$(dirname "$QEMU")/qemu-build-kf3}"
[ -f "$QEMU/VERSION" ] || { echo "⊘ $QEMU is not a QEMU source tree"; exit 1; }
command -v cargo >/dev/null || { echo "⊘ cargo not on PATH (export PATH=\$HOME/.cargo/bin:\$PATH)"; exit 127; }
echo "== kf3 build: QEMU $(cat "$QEMU/VERSION") rev $(git -C "$REPO" rev-parse --short=8 HEAD)"
( cd "$REPO" && cargo build --release -p kf-qemu )
ARCHIVE="$REPO/target/release/libkf_qemu.a"
[ -f "$ARCHIVE" ] || { echo "⊘ no archive at $ARCHIVE"; exit 1; }
[ -z "$(find "$ARCHIVE" -mmin +30)" ] || { echo "⊘ $ARCHIVE is >30 min old — cargo did not rebuild it"; exit 1; }
mkdir -p "$QEMU/hw/misc/kf3"
cp "$REPO"/qemu/hw/misc/kf3/kf3.c "$REPO"/qemu/hw/misc/kf3/kf3.h "$REPO"/qemu/hw/misc/kf3/meson.build "$QEMU/hw/misc/kf3/"
cp "$ARCHIVE" "$QEMU/hw/misc/kf3/libkf_qemu.a"
grep -q "subdir('kf3')" "$QEMU/hw/misc/meson.build" || printf "\nsubdir('kf3')\n" >> "$QEMU/hw/misc/meson.build"
grep -q '^config KF3' "$QEMU/hw/misc/Kconfig" || printf '\nconfig KF3\n    bool\n    default y if TEST_DEVICES\n    depends on PCI\n' >> "$QEMU/hw/misc/Kconfig"
if [ ! -f "$BUILD/build.ninja" ]; then
  mkdir -p "$BUILD"
  ( cd "$BUILD" && "$QEMU/configure" --target-list=x86_64-softmmu \
      --disable-docs --disable-tools --disable-guest-agent --disable-werror --disable-slirp \
      --disable-vnc --disable-gtk --disable-sdl --disable-curses --disable-libssh --disable-vde \
      --disable-tpm --without-default-features --enable-kvm --enable-system >/dev/null )
fi
ninja -C "$BUILD" qemu-system-x86_64
echo "KF3_BUILT $BUILD/qemu-system-x86_64 rev=$(git -C "$REPO" rev-parse --short=8 HEAD) mtime=$(stat -c %y "$BUILD/qemu-system-x86_64")"
