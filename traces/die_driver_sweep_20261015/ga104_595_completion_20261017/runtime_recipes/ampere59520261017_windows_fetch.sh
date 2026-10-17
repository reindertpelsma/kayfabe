#!/usr/bin/env bash
set -euo pipefail
exec >/root/prov/ampere595_windows_fetch.log 2>&1
trap 'rc=$?; echo "WINDOWS_FETCH_EXIT rc=$rc $(date -u -Is)"' EXIT
test "$(cat /root/prov/OWN_SWEEP_ID)" = 56326220
echo "WINDOWS_FETCH_START $(date -u -Is)"
export WINVM_ROOT=/workspace/ampere-winvm WINVM_NAME=ampere580
timeout --signal=TERM 1200 bash /root/kayfabe/scripts/bench/windows/win_vm.sh fetch
