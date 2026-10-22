#!/bin/bash
set -euo pipefail
cd /root/kayfabe
exec >/root/prov/sweep_windows_media.log 2>&1
trap 'rc=$?; echo "WIN_MEDIA_EXIT rc=$rc $(date -u -Is)"' EXIT
echo "WIN_MEDIA_START $(date -u -Is)"
export WINVM_ROOT=/workspace/sweep-win-57144505
bash scripts/bench/windows/win_vm.sh fetch
