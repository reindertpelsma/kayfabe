#!/usr/bin/env bash
set -euo pipefail
W=/var/lib/kf-windows-20261005
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_prod3.sh
R=$W/boundary-kayfabe-647
REV=2c917f3e
cleanup(){ rc=$?; trap - EXIT; bash "$B" stop 647 || true; bash "$W/pti-iommu_nogdm.sh" DMA-FQ; rm -f "$W/kf3-bins/$REV" /tmp/kf-stop-boundary-647; echo BOUNDARY647_EXIT=$rc; }
if pgrep qemu-system-x86; then exit 1; fi
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
test ! -e "$W/kf3-bins/$REV"
ln -s "$W/readtrace/kf3-bins/$REV" "$W/kf3-bins/$REV"
trap cleanup EXIT
export KF3_REV=$REV WIN_FLAGS=KF3_RPC_TRACE=1 WIN_TRACE=1 WIN_GSP_OBSERVER=1
bash "$W/pti-iommu_nogdm.sh" identity
WIN_REUSE=0 bash "$B" run 647
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
echo BOUNDARY647_READY="$(date -Is)"
python3 /tmp/kf-stage-generic.py "$R" > "$W/codex-boundary647-stage.txt"
sleep 35
G qga-exec powershell.exe -NoProfile -Command 'Get-Content C:\kf\stage-out.txt' > "$W/codex-boundary647-stage-out.txt"
G qga-exec powershell.exe -NoProfile -Command 'Get-Content C:\kf\io-stage.txt' > "$W/codex-boundary647-io.txt"
echo BOUNDARY647_PROBE_COMPLETE="$(date -Is)"
while [ ! -e /tmp/kf-stop-boundary-647 ] && kill -0 "$(sed -n 's/^QPID=//p' "$W/windows-broker-run647.state")"; do sleep 5; done
