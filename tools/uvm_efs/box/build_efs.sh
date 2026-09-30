#!/usr/bin/env bash
# tools/uvm_efs/box/build_efs.sh — build nvidia-uvm.ko with the EFS patch, on the box.
#   bash build_efs.sh <patch-file> [outdir]      (default outdir /root/efs/patched)
# Copies the installed DKMS source tree (provision.sh proved it IS ogkm 580.159.04's
# kernel-open), applies the patch to nvidia-uvm/, builds nvidia + nvidia-uvm together (so
# nvidia-uvm's symbol CRCs come from the same tree as the loaded nvidia.ko), and prints
# BUILD_RC / PATCH_SHA / KO_SHA. Loads nothing: load_efs.sh does that.
set -uo pipefail
PATCH=${1:?usage: build_efs.sh <patch> [outdir]}
OUT=${2:-/root/efs/patched}
SRC=/usr/src/nvidia-580.159.04
echo "BUILD_START $(date -Is) patch=$PATCH sha=$(sha256sum "$PATCH" | cut -c1-16)"
rm -rf "$OUT" && cp -a "$SRC" "$OUT" || { echo "BUILD_RC=90 (copy)"; exit 90; }
( cd "$OUT" && patch -p1 --no-backup-if-mismatch < "$PATCH" ) > "$OUT.patch.log" 2>&1
PRC=$?; echo "PATCH_RC=$PRC"; [ $PRC -eq 0 ] || { cat "$OUT.patch.log"; echo "BUILD_RC=91"; exit 91; }
( cd "$OUT" && make -j"$(nproc)" modules SYSSRC="/lib/modules/$(uname -r)/build" \
      NV_KERNEL_MODULES="nvidia nvidia-uvm" ) > "$OUT.build.log" 2>&1
BRC=$?
grep -a -E "error|warning: .*uvm_efs|uvm_efs.*warning" "$OUT.build.log" | head -40
echo "BUILD_RC=$BRC"
if [ $BRC -eq 0 ]; then
  echo "KO_SHA=$(sha256sum "$OUT/nvidia-uvm.ko" | cut -c1-16)"
  modinfo "$OUT/nvidia-uvm.ko" | grep -E "^(version|vermagic)|parm: *uvm_efs"
fi
echo "BUILD_EXIT $(date -Is)"
exit $BRC
