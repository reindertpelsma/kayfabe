#!/usr/bin/env bash
# build_qemu.sh — clone QEMU 9.2, patch virtio-nvgpu into it, and build a
#                 minimal KVM-only QEMU binary at /opt/qemu-nvkvm.
#
# Idempotent: if /opt/qemu-nvkvm/bin/qemu-system-x86_64 already exists the
# script prints a message and exits successfully.

set -euo pipefail

QEMU_VERSION="9.2.0"
QEMU_SRC="/opt/qemu-src"
QEMU_PREFIX="/opt/qemu-nvkvm"
REPO_ROOT="$(realpath "$(dirname "$0")/..")"

# ── Guard: already built ───────────────────────────────────────────────────
if [ -x "$QEMU_PREFIX/bin/qemu-system-x86_64" ]; then
    echo "INFO: $QEMU_PREFIX/bin/qemu-system-x86_64 already exists — skipping build."
    exit 0
fi

echo "=== nvkvm QEMU build ==="
echo "QEMU version : $QEMU_VERSION"
echo "Source tree  : $QEMU_SRC"
echo "Install path : $QEMU_PREFIX"
echo ""

# ── 1. Install build dependencies ─────────────────────────────────────────
echo "[1/9] Installing build dependencies..."
apt-get update -q
apt-get install -y \
    ninja-build \
    meson \
    libglib2.0-dev \
    libpixman-1-dev \
    python3 \
    git \
    libslirp-dev \
    pkg-config \
    libattr1-dev

# ── 2. Clone QEMU 9.2 stable ──────────────────────────────────────────────
if [ ! -d "$QEMU_SRC" ]; then
    echo "[2/9] Cloning QEMU $QEMU_VERSION..."
    git clone --depth=1 --branch "v${QEMU_VERSION}" \
        https://gitlab.com/qemu-project/qemu.git "$QEMU_SRC"
else
    echo "[2/9] QEMU source already present at $QEMU_SRC — skipping clone."
fi

# ── 3. Copy nvkvm QEMU source files into hw/misc/ ─────────────────────────
echo "[3/9] Copying nvkvm QEMU source files to $QEMU_SRC/hw/misc/..."
cp "$REPO_ROOT/src/qemu/"*.c "$QEMU_SRC/hw/misc/"
cp "$REPO_ROOT/src/qemu/"*.h "$QEMU_SRC/hw/misc/"

# ── 4. Copy ABI / common headers into hw/misc/nvkvm_inc/ ──────────────────
echo "[4/9] Copying ABI and common headers to $QEMU_SRC/hw/misc/nvkvm_inc/..."
mkdir -p "$QEMU_SRC/hw/misc/nvkvm_inc"
cp "$REPO_ROOT/src/abi/nvgpu.h"          "$QEMU_SRC/hw/misc/nvkvm_inc/"
cp "$REPO_ROOT/src/abi/uvm.h"            "$QEMU_SRC/hw/misc/nvkvm_inc/"
cp "$REPO_ROOT/src/common/nvkvm_proto.h" "$QEMU_SRC/hw/misc/nvkvm_inc/"
# Linux type shim: replaces <linux/types.h> in the QEMU user-space build
# to avoid conflicts with QEMU's own type setup in qemu/osdep.h.
cp "$REPO_ROOT/src/qemu/nvkvm_linux_types.h" \
   "$QEMU_SRC/hw/misc/nvkvm_inc/linux_types_compat.h"

# ── 5. Fix include paths in the copied files ──────────────────────────────
echo "[5/9] Fixing include paths in copied files..."
# virtio_nvgpu.h uses relative paths like ../../src/common/nvkvm_proto.h
# that are correct relative to src/qemu/ but wrong inside hw/misc/.
# Rewrite them to use the local nvkvm_inc/ sub-directory.
sed -i \
    's|"../../src/common/nvkvm_proto.h"|"nvkvm_inc/nvkvm_proto.h"|g' \
    "$QEMU_SRC/hw/misc/virtio_nvgpu.h"
sed -i \
    's|"../../src/abi/nvgpu.h"|"nvkvm_inc/nvgpu.h"|g' \
    "$QEMU_SRC/hw/misc/virtio_nvgpu.h"
sed -i \
    's|"../../src/abi/uvm.h"|"nvkvm_inc/uvm.h"|g' \
    "$QEMU_SRC/hw/misc/virtio_nvgpu.h"
# Replace <linux/types.h> in nvkvm_inc headers with our QEMU-compatible shim
# to avoid conflicts with QEMU's own qemu/osdep.h type setup.
sed -i \
    's|#include <linux/types.h>|#include "linux_types_compat.h"|g' \
    "$QEMU_SRC/hw/misc/nvkvm_inc/"*.h

# ── 6. Patch hw/misc/meson.build ─────────────────────────────────────────
echo "[6/9] Patching $QEMU_SRC/hw/misc/meson.build..."

MESON_BUILD="$QEMU_SRC/hw/misc/meson.build"

# Only patch once (idempotent).
if ! grep -q 'virtio_nvgpu.c' "$MESON_BUILD"; then
    # Insert the nvkvm block before the final line of the file.
    # We use a Python one-liner to keep things portable and avoid sed
    # multi-line headaches.
    python3 - "$MESON_BUILD" <<'PYEOF'
import sys

path = sys.argv[1]
with open(path, 'r') as fh:
    lines = fh.readlines()

nvkvm_block = """\

nvkvm_inc = include_directories('nvkvm_inc')

system_ss.add(when: ['CONFIG_VIRTIO'], if_true: files(
  'virtio_nvgpu.c',
  'nvkvm_dispatch.c',
  'nvkvm_frontend.c',
  'nvkvm_objects.c',
  'nvkvm_mmap_host.c',
), extra_args: ['-I' + meson.current_source_dir() + '/nvkvm_inc'])

# Mode-2 emulated NVIDIA GPU PCI device (reverse driver). Not virtio; plain
# PCI device, always built into the x86_64 softmmu target.
system_ss.add(when: ['CONFIG_PCI'], if_true: files(
  'nvkvm_gpu_emul.c',
  'nvkvm_m2_rec.c',
))
"""

# Insert the block before the very last non-empty line.
insert_pos = len(lines)
for i in range(len(lines) - 1, -1, -1):
    if lines[i].strip():
        insert_pos = i
        break

lines.insert(insert_pos, nvkvm_block)

with open(path, 'w') as fh:
    fh.writelines(lines)

print("  meson.build patched successfully.")
PYEOF
else
    echo "  meson.build already contains virtio_nvgpu.c — skipping patch."
fi

# #90: a tree patched before the replay-trace recorder existed has the old
# one-file PCI block.  The guard above is on virtio_nvgpu.c, so it will never
# re-run — add the recorder separately, idempotently.  Without this an
# already-provisioned bench links with an undefined nvkvm_rec_emit.
if ! grep -q "nvkvm_m2_rec.c" "$MESON_BUILD"; then
    echo "  adding nvkvm_m2_rec.c to the PCI block..."
    python3 - "$MESON_BUILD" <<'PYEOF'
import sys
path = sys.argv[1]
lines = open(path).readlines()
# Anchor on the emulator's own entry wherever it sits — provisioned benches have
# hand-extended this block, so an exact whole-block match would not survive.
for i, l in enumerate(lines):
    if l.strip() == "'nvkvm_gpu_emul.c',":
        lines.insert(i + 1, l.replace('nvkvm_gpu_emul.c', 'nvkvm_m2_rec.c'))
        open(path, 'w').writelines(lines)
        print("  nvkvm_m2_rec.c added after nvkvm_gpu_emul.c.")
        break
else:
    sys.exit("FATAL: no 'nvkvm_gpu_emul.c' entry to anchor on in " + path)
PYEOF
fi

# ── 7. Configure QEMU ─────────────────────────────────────────────────────
echo "[7/9] Configuring QEMU (target: x86_64-softmmu, KVM only)..."
cd "$QEMU_SRC"
./configure \
    --target-list=x86_64-softmmu \
    --enable-kvm \
    --disable-werror \
    --disable-sdl \
    --disable-gtk \
    --disable-opengl \
    --disable-virglrenderer \
    --disable-vnc \
    --prefix="$QEMU_PREFIX"

# ── 8. Build ──────────────────────────────────────────────────────────────
echo "[8/9] Building QEMU with ninja -j$(nproc)..."
ninja -j"$(nproc)"

# ── 9. Install ────────────────────────────────────────────────────────────
echo "[9/9] Installing to $QEMU_PREFIX..."
ninja install

echo ""
echo "=== Build complete ==="
echo "Binary: $QEMU_PREFIX/bin/qemu-system-x86_64"
