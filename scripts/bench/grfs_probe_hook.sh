#!/usr/bin/env bash
# ★ POST_CAPTURE_HOOK for `boot_capture.sh`: run `probes/grfs_probe.c` INSIDE the fat guest,
# against the guest's OWN RM — the other half of the host probe (`traces/real_ga104/`).
#
#   KF_DEVICE=kf3 POST_CAPTURE_HOOK=$PWD/scripts/bench/grfs_probe_hook.sh \
#       bash scripts/bench/boot_capture.sh <tag>
#
# The guest's RM answers its userspace's GR floorsweeping controls (GR_GET_GPC_MASK,
# GR_GET_TPC_MASK, GR_GET_NUM_TPCS_FOR_GPC, …) from the replies kayfabe served it, and forwards
# GRMGR_GET_GR_FS_INFO to kayfabe; so the same program on both sides, diffed CTRL by CTRL, is the
# end-to-end check that the guest sees what the host reports (v3-gpcmask).
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
G="$SRC_DIR/gssh_nv"
SRC="$SRC_DIR/probes/grfs_probe.c"
echo "=== grfs_probe source md5 $(md5sum < "$SRC" | cut -d' ' -f1) ==="
$G 'cat > /tmp/grfs_probe.c' < "$SRC" || { echo "★ grfs_probe hook: could not push the source"; exit 2; }
$G 'rm -f /tmp/grfs_probe; gcc -O1 -o /tmp/grfs_probe /tmp/grfs_probe.c 2>&1; echo GCC_RC=$?'
$G 'test -x /tmp/grfs_probe' || { echo "★ grfs_probe hook: no binary in the guest"; exit 2; }
echo "=== GUEST grfs_probe (the guest's root, the guest's /dev/nvidia0) ==="
$G 'sudo /tmp/grfs_probe 0 0 2>&1; echo GUEST_PROBE_RC=$?'
echo "=== GUEST grfs_probe done ==="
