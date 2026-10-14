#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_prod3.sh
R=$W/boundary-kayfabe-648
REV=2c917f3e
cleanup(){ rc=$?; trap - EXIT; bash "$B" stop 648 || true; bash "$W/pti-iommu_nogdm.sh" DMA-FQ; rm -f "$W/kf3-bins/$REV" /tmp/kf-stop-boundary-648; echo BOUNDARY648_EXIT=$rc; }
if pgrep qemu-system-x86; then exit 1; fi
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
test ! -e "$W/kf3-bins/$REV"
ln -s "$W/readtrace/kf3-bins/$REV" "$W/kf3-bins/$REV"
trap cleanup EXIT
export KF3_REV=$REV WIN_FLAGS= WIN_TRACE=0 WIN_GSP_OBSERVER=0
bash "$W/pti-iommu_nogdm.sh" identity
WIN_REUSE=0 bash "$B" run 648
G(){ timeout 40 python3 "$W/boundary-tools/qmp.py" "$R/qga.sock" "$@"; }
for i in $(seq 90); do G qga-ping >/dev/null 2>&1 && break; sleep 2; done
IFS= read -r KF_GUEST_PW < "$W/windows-desktop/secrets/win_password" || test -n "$KF_GUEST_PW"
export KF_GUEST_PW
G qga-exec net.exe user vast "$KF_GUEST_PW" >/dev/null
G qga-exec powershell.exe -NoProfile -Command 'New-ItemProperty -Path "HKLM:\SOFTWARE\Microsoft\Windows\Dwm" -Name OverlayTestMode -Value 5 -PropertyType DWord -Force | Out-Null'
sleep 60
signed=0
for attempt in 1 2 3; do
 if python3 /data/kf-gl730101-host/scripts/bench/windows/signin.py "$R/qmp.sock" "$R/qga.sock"; then signed=1; break; fi
 echo SIGNIN_RETRY=$attempt
 sleep 10
done
test $signed = 1
echo BOUNDARY648_READY="$(date -Is)"
python3 /tmp/kf-stage-normal.py "$R" > "$W/codex-boundary648-stage.txt"
sleep 50
G qga-exec powershell.exe -NoProfile -Command 'Get-Content C:\kf\stage-out.txt' > "$W/codex-boundary648-stage-out.txt"
G qga-exec powershell.exe -NoProfile -Command 'Get-CimInstance Win32_PnPEntity | Where-Object Name -Like "*NVIDIA*" | Select-Object Name,ConfigManagerErrorCode' > "$W/codex-boundary648-adapter.txt"
echo BOUNDARY648_PROBE_COMPLETE="$(date -Is)"
grep -q 'KFGL RESULT OK' "$W/codex-boundary648-stage-out.txt"
grep -q '^EXIT 0' "$W/codex-boundary648-stage-out.txt"
echo NORMAL_PROFILE_GEARS_PASS
