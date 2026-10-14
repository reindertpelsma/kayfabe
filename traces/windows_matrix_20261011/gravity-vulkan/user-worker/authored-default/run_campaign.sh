#!/usr/bin/env bash
# Trusted host; caller holds /tmp/kayfabe-fastguest.lock with flock -o.
set -euo pipefail
REV=${1:?full source revision required}
BASE=${2:?unused Windows run base required}
[[ "$REV" =~ ^[0-9a-f]{40}$ ]]
[[ "$BASE" =~ ^[0-9]+$ ]]
TREE=/data/kf-memory-access-host-${REV:0:9}
ROOT=/var/lib/kf-windows-20261005/codex-authored-default-${REV:0:9}
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
test ! -e "$ROOT"
mkdir "$ROOT"
date -Is > "$ROOT/START"
trap 'rc=$?; echo "$rc" > "$ROOT/EXIT"; echo AUTHORED_DEFAULT_CAMPAIGN_EXIT=$rc' EXIT
echo 'H-IDENTITY: explicitly authored native/scanner bit0 removes the guest-default0 admission refusal.'
echo 'FALSIFIER zero-ID refusal or native/scanner mask divergence. Native zero/id1 fixture is host-specific evidence, not guest initialization proof.'
bash "$TREE/traces/windows_matrix_20261011/gravity-vulkan/user-memory-access/run_linux.sh" "$REV"
bash "$TREE/traces/windows_matrix_20261011/gravity-vulkan/user-worker/run_linux_desktop.sh" "$REV"
bash "$TREE/traces/windows_matrix_20261011/gravity-vulkan/user-worker/run_windows.sh" "$REV" "$BASE"
