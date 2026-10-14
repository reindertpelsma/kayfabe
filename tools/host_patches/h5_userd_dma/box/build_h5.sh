#!/usr/bin/env bash
# tools/host_patches/h5_userd_dma/box/build_h5.sh — BUILD-ONLY: apply the H5 patch to a copy of an
# NVIDIA open-module source tree and compile the modules. Loads, installs and replaces NOTHING.
#
#   bash build_h5.sh <src-tree> <patch> <outdir> [kernel-release]
#
#   <src-tree>  a packaged DKMS tree (/usr/src/nvidia-<ver>, has nv-kernel.o_binary) OR the
#               kernel-open/ directory of an open-gpu-kernel-modules checkout (needs the whole
#               checkout's src/ for the RM core; pass the checkout ROOT in that case)
#   <patch>     tools/host_patches/h5_userd_dma/patch/nvidia_h5_userd_dma_<ver>.patch
#   <outdir>    scratch directory OUTSIDE any live driver directory; it is deleted first
#   [kernel]    default: uname -r. Needs /lib/modules/<kernel>/build (kernel headers).
#
# Prints PATCH_RC / BUILD_RC / KO_SHA and the modinfo lines that matter. Exit code: the build's.
set -uo pipefail
SRC=${1:?usage: build_h5.sh <src-tree> <patch> <outdir> [kernel]}
PATCH=${2:?patch}
OUT=${3:?outdir}
KREL=${4:-$(uname -r)}
case "$OUT" in /usr/src/*|/lib/modules/*|/var/lib/*|/opt/*) echo "refusing outdir $OUT (live path)"; exit 2;; esac
[ -d "/lib/modules/$KREL/build" ] || { echo "no kernel headers for $KREL"; exit 3; }
echo "BUILD_START $(date -Is) kernel=$KREL patch_sha=$(sha256sum "$PATCH" | cut -c1-16)"
rm -rf "$OUT" && mkdir -p "$OUT" && cp -a "$SRC"/. "$OUT"/ || { echo "BUILD_RC=90 (copy)"; exit 90; }
# A checkout root has kernel-open/; a packaged DKMS tree is already the kernel-open layout.
if [ -d "$OUT/kernel-open" ]; then TREE="$OUT/kernel-open"; BUILD="$OUT"; else TREE="$OUT"; BUILD="$OUT"; fi
( cd "$TREE" && patch -p1 --no-backup-if-mismatch < "$PATCH" ) > "$OUT.patch.log" 2>&1
PRC=$?; echo "PATCH_RC=$PRC"; [ $PRC -eq 0 ] || { cat "$OUT.patch.log"; echo "BUILD_RC=91"; exit 91; }
# Same make line as the packaged dkms.conf, minus the install.
( cd "$BUILD" && unset ARCH && make -j"${JOBS:-$(nproc)}" NV_EXCLUDE_BUILD_MODULES='' \
    KERNEL_UNAME="$KREL" IGNORE_XEN_PRESENCE=1 IGNORE_CC_MISMATCH=1 \
    SYSSRC="/lib/modules/$KREL/build" LD=/usr/bin/ld.bfd CONFIG_X86_KERNEL_IBT= modules ) \
    > "$OUT.build.log" 2>&1
BRC=$?
grep -a -E "error:|nv\.c:[0-9]+.*warning|nv-kf.*warning" "$OUT.build.log" | head -40
echo "BUILD_RC=$BRC"
if [ $BRC -eq 0 ]; then
  for m in nvidia nvidia-uvm nvidia-modeset nvidia-drm; do
    k=$(find "$BUILD" -maxdepth 2 -name "$m.ko" | head -1)
    [ -n "$k" ] && echo "KO_SHA $m $(sha256sum "$k" | cut -c1-16) $(stat -c %s "$k")"
  done
  k=$(find "$BUILD" -maxdepth 2 -name nvidia.ko | head -1)
  modinfo "$k" | grep -E "^(version|vermagic|license)|parm: *kf_dma_window"
fi
echo "BUILD_EXIT $(date -Is)"
exit $BRC
