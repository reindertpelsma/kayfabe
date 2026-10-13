#!/usr/bin/env bash
set -euo pipefail
REV=6ff5aaf293e9fc04ac8780983f26dc04d0f3b735
TREE=/data/kf-memory-access-host-6ff5aaf29
OUT=/var/lib/kf-windows-20261005/codex-user-worker-6ff5aaf29
mkdir -p "$OUT"
date -Is > "$OUT/START"
trap 'rc=$?; echo "$rc" > "$OUT/EXIT"; echo USER_WORKER_LINUX_EXIT=$rc' EXIT
python3 /tmp/kf-userd-resolved-cleanup-1550.py
bash "$TREE/traces/windows_matrix_20261011/gravity-vulkan/user-memory-access/run_linux.sh" "$REV"
bash "$TREE/traces/windows_matrix_20261011/gravity-vulkan/user-worker/run_linux_desktop.sh" "$REV"
