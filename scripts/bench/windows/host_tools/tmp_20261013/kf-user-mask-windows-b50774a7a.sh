#!/usr/bin/env bash
set -euo pipefail
REV=b50774a7a5343cf10af849db1445b577e9b21b3f
TREE=/data/kf-memory-access-host-b50774a7a
W=/var/lib/kf-windows-20261005
test "$(cat "$W/codex-user-worker-b50774a7a/EXIT")" = 0
python3 /tmp/kf-user-mask-cleanup-1540.py
echo 'HYPOTHESIS public subdevice filtering with matched native identity preserves Windows desktop progress and original GravityMark Vulkan workload.'
echo 'FALSIFIER mask/source refusal, missing progress, device loss, wrong output or hostXid32/guest141/4101. Native mask2/2 and exact Linux9+10/fast30/desktop must precede this run.'
echo 'ONE_BEHAVIOR_VARIABLE public subdevice mask semantics/identity relative to1590; production flags, original600s oracle/media and source budget unchanged.'
bash "$TREE/traces/windows_matrix_20261011/gravity-vulkan/user-worker/run_windows.sh" "$REV" 1600
