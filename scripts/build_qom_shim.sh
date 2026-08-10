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
# ★ `KAYFABE_SHIM_FEATURES` names cargo features for the archive, space-separated.
# `execution_plane_increments.md` E0's evidence build uses `host-isolates`; the default is
# EMPTY, so a bench that does not set it gets byte-for-byte the archive master ships.
# ⊘ It is not a boolean and there is no default-on list: a feature that arrived silently in
# a hypervisor archive would be a host-side capability nobody chose.
FEATURES="${KAYFABE_SHIM_FEATURES:-}"
FEATARGS=()
if [ -n "$FEATURES" ]; then
  FEATARGS=(--features "$(echo "$FEATURES" | tr ' ' ',')")
  echo "== archive features: $FEATURES"
fi
echo "== building the archive"
( cd "$REPO" && cargo build --release -p kayfabe-qemu-raw "${FEATARGS[@]}" )
# ★★★ ASK CARGO WHERE IT PUT THE ARCHIVE — never assume `$REPO/target`.
#
# `[measured 2026-08-10, bench `vh`]` this line read `ARCHIVE="$REPO/target/release/…"`.
# A build invoked as `CARGO_TARGET_DIR=/workspace/bench/cargo-target … build_qom_shim.sh`
# compiled the archive **with `host-isolates`** into the target dir it was told to use, and
# then copied a DIFFERENT, older, feature-less archive out of `$REPO/target/release/` —
# which still existed from an earlier build. Every line of the build log was green:
# `== archive features: host-isolates`, `Finished release profile in 20.87s`, `== built:`.
# The only signal was the rev stamp inside the two artifacts naming the PREVIOUS commit.
#
# ⊘ That is `bench_origin_is_a_bundle_and_cargo_is_not_on_path` one layer in: the build ran,
# succeeded, and installed something nobody chose. Two assertions close it — locate the
# archive by asking cargo, and refuse to install one that is older than the build.
ARCHIVE=$(
  cd "$REPO" && cargo metadata --format-version 1 --no-deps 2>/dev/null |
    sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p'
)
ARCHIVE="${ARCHIVE:-$REPO/target}/release/libkayfabe_qemu_raw.a"
[ -f "$ARCHIVE" ] || { echo "★ no archive at $ARCHIVE"; exit 1; }
# ★ And it must be THIS build's. A stale archive at the right path is the same defect with
# the path bug fixed, so the freshness is asserted rather than inferred from the log.
if [ -n "$(find "$ARCHIVE" -mmin +30 2>/dev/null)" ]; then
  echo "★ $ARCHIVE is more than 30 minutes old — cargo did not rebuild it and this"
  echo "  script will not install an archive it did not just produce."
  exit 1
fi
echo "== archive: $ARCHIVE ($(stat -c %y "$ARCHIVE" 2>/dev/null))"

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
