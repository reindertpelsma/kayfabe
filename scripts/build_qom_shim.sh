#!/usr/bin/env bash
# Lay the QOM shim overlay into a hypervisor source tree and build it.
#
# ★★ DECISION Q1's shape, made executable: the overlay is a DIRECTORY the user drops in, plus
# exactly TWO hunks. This script is the whole install story, and its length is the honest
# measure of the "unpaid cost" `l2_qemu_adapter.md` §2.1 records — a user builds a hypervisor
# once, because there is no supported out-of-tree device mechanism at any release we target.
#
#   usage: scripts/build_qom_shim.sh <qemu-source-tree> [<build-dir>]
#
# Idempotent: re-running against an already-patched tree re-syncs the overlay and rebuilds.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
QEMU="${1:?usage: build_qom_shim.sh <qemu-source-tree> [<build-dir>]}"
BUILD="${2:-$QEMU/build-nvkvm}"
QEMU="$(cd "$QEMU" && pwd)"

[ -f "$QEMU/VERSION" ] || { echo "★ $QEMU is not a hypervisor source tree"; exit 1; }
echo "== target: $QEMU ($(cat "$QEMU/VERSION"))"

# ---- 1. the Rust archive ------------------------------------------------------------
# ★ Built in release, because a debug archive drags the whole standard library's assertions
# into a hypervisor's link and the difference is measured in tens of megabytes.
echo "== building the archive"
( cd "$REPO" && cargo build --release -p kayfabe-qemu-raw )
ARCHIVE="$REPO/target/release/libkayfabe_qemu_raw.a"
[ -f "$ARCHIVE" ] || { echo "★ no archive at $ARCHIVE"; exit 1; }

# ---- 2. the overlay -----------------------------------------------------------------
echo "== laying the overlay into hw/misc/nvkvm"
mkdir -p "$QEMU/hw/misc/nvkvm"
cp "$REPO"/qemu/hw/misc/nvkvm/*.c "$REPO"/qemu/hw/misc/nvkvm/*.h \
   "$REPO"/qemu/hw/misc/nvkvm/meson.build "$QEMU/hw/misc/nvkvm/"
cp "$ARCHIVE" "$QEMU/hw/misc/nvkvm/libkayfabe_qemu_raw.a"

# ---- 3. hunk one: hw/misc/meson.build ----------------------------------------------
if ! grep -q "subdir('nvkvm')" "$QEMU/hw/misc/meson.build"; then
  echo "== hunk 1/2: hw/misc/meson.build"
  printf "\nsubdir('nvkvm')\n" >> "$QEMU/hw/misc/meson.build"
fi

# ---- 4. hunk two: hw/misc/Kconfig ---------------------------------------------------
if ! grep -q '^config NVKVM' "$QEMU/hw/misc/Kconfig"; then
  echo "== hunk 2/2: hw/misc/Kconfig"
  cat >> "$QEMU/hw/misc/Kconfig" <<'EOF'

config NVKVM
    bool
    default y if TEST_DEVICES
    depends on PCI
EOF
fi

# ---- 5. configure + build -----------------------------------------------------------
if [ ! -f "$BUILD/build.ninja" ]; then
  echo "== configuring $BUILD"
  mkdir -p "$BUILD"
  ( cd "$BUILD" && "$QEMU/configure" \
      --target-list=x86_64-softmmu \
      --disable-docs --disable-tools --disable-guest-agent \
      --disable-werror --disable-slirp --disable-vnc --disable-gtk --disable-sdl \
      --disable-curses --disable-libssh --disable-vde --disable-tpm \
      --without-default-features --enable-kvm --enable-system \
      >/dev/null )
fi

echo "== building"
ninja -C "$BUILD" qemu-system-x86_64
echo "== built: $BUILD/qemu-system-x86_64"
