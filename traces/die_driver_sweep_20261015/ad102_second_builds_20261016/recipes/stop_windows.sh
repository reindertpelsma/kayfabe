#!/usr/bin/env bash
set -euo pipefail
run=${1:?owned run};rev=${2:?private binary revision};repo=${3:?owned repo}
test "$run" = /root/win/codex-ada2build
[[ "$rev" =~ ^[0-9a-f]{8}$ ]]
exec >"/root/prov/ada2build_stop_$rev.log" 2>&1
trap 'rc=$?; echo "STOP_EXIT rc=$rc $(date -u -Is)"' EXIT
pid=$(cat "$run/qpid");exe=$(readlink -f "/proc/$pid/exe")
test "$exe" = /workspace/bench/kf3-bins/$rev/qemu-system-x86_64
tr '\0' '\n' <"/proc/$pid/cmdline" | grep -Fx "unix:$run/qmp.sock,server=on,wait=off"
echo "STOP_START pid=$pid $(date -u -Is)"
timeout 20 python3 "$repo/scripts/bench/windows/qmp.py" "$run/qga.sock" qga-exec shutdown.exe /s /f /t 0 || true
for _ in $(seq 60); do kill -0 "$pid" 2>/dev/null || break; sleep 1; done
if kill -0 "$pid" 2>/dev/null; then
 timeout 10 python3 "$repo/scripts/bench/windows/qmp.py" "$run/qmp.sock" cmd system_powerdown || true
 for _ in $(seq 60); do kill -0 "$pid" 2>/dev/null || break; sleep 1; done
fi
if kill -0 "$pid" 2>/dev/null; then
 test "$(readlink -f "/proc/$pid/exe")" = "$exe"
 kill -TERM "$pid"
 for _ in $(seq 15); do kill -0 "$pid" 2>/dev/null || break; sleep 1; done
fi
! kill -0 "$pid" 2>/dev/null
rm -f "$run/qpid"
echo OWN_STOP_COMPLETE=1
