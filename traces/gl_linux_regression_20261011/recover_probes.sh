#!/usr/bin/env bash
# Recovery used in run 642: runner paused with SIGSTOP while its QEMU and outer flock stayed live.
set -euo pipefail
N=${1:?run number}
[[ "$N" =~ ^[0-9]+$ ]]
W=/var/lib/kf-windows-20261005
REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
RUN=$W/boundary-kayfabe-$N
OUT=$W/winprod/run$N
IFS= read -r KF_GUEST_PW < "$W/windows-desktop/secrets/win_password" || test -n "$KF_GUEST_PW"
export KF_GUEST_PW
python3 "$REPO/scripts/bench/windows/signin.py" "$RUN/qmp.sock" "$RUN/qga.sock" > "$OUT/signin-recovery.txt" 2>&1
python3 "$W/boundary-tools/qmp.py" "$RUN/qga.sock" qga-exec powershell.exe -NoProfile -Command \
    '$u=(Get-CimInstance Win32_ComputerSystem).UserName; if (-not $u) { throw "No interactive user" }; schtasks /delete /tn kfgl /f 2>&1 | Out-Null; schtasks /create /tn kfgl /tr "powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\kf\kfgl.ps1" /sc once /st 23:59 /ru $u /it /f 2>&1; schtasks /run /tn kfgl 2>&1' > "$OUT/probe-recovery.txt" 2>&1
runner=$(cat "$OUT/runner.pid")
case "$(ps -p "$runner" -o args=)" in
    *tdr-run28.sh\ "$N") kill -CONT "$runner" ;;
    *) echo "Runner identity changed; not resumed" >&2; exit 1 ;;
esac
