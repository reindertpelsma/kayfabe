#!/usr/bin/env bash
set -euo pipefail
REV=30c887c8651c3e79b6b62c6785eda30f443205d1
TREE=/data/kf-memory-access-host-30c887c86
OUT=/var/lib/kf-windows-20261005/codex-user-worker-30c887c86
mkdir -p "$OUT"
date -Is > "$OUT/START"
trap 'rc=$?; echo "$rc" > "$OUT/EXIT"; echo USER_WORKER_LINUX_EXIT=$rc' EXIT
bash "$TREE/traces/windows_matrix_20261011/gravity-vulkan/user-memory-access/run_linux.sh" "$REV"
bash "$TREE/traces/windows_matrix_20261011/gravity-vulkan/user-worker/run_linux_desktop.sh" "$REV"
