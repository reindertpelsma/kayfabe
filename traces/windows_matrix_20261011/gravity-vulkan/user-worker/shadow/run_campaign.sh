#!/usr/bin/env bash
# Trusted host only, entire campaign under global flock -o.
set -euo pipefail
REV=${1:?full source revision}
BASE=${2:?unused Windows base}
[[ "$REV" =~ ^[0-9a-f]{40}$ ]]
TREE=/data/kf-memory-access-host-${REV:0:9}
ROOT=/var/lib/kf-windows-20261005/codex-user-shadow-${REV:0:9}
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
test ! -e "$ROOT"
mkdir -p "$ROOT"
date -Is > "$ROOT/START"
trap 'rc=$?; echo "$rc" > "$ROOT/EXIT"; echo USER_SHADOW_CAMPAIGN_EXIT=$rc' EXIT
bash "$TREE/traces/windows_matrix_20261011/gravity-vulkan/user-memory-access/run_linux.sh" "$REV"
bash "$TREE/traces/windows_matrix_20261011/gravity-vulkan/user-worker/run_linux_desktop.sh" "$REV"
bash "$TREE/traces/windows_matrix_20261011/gravity-vulkan/user-worker/shadow/run_windows.sh" "$REV" "$BASE"
