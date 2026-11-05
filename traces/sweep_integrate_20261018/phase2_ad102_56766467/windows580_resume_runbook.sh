#!/usr/bin/env bash
set -euo pipefail
export PATH=/root/.cargo/bin:$PATH
cd /root/kayfabe
exec >/root/prov/sweep_windows580_resume.log 2>&1
trap 'rc=$?; echo "WINDOWS580_RESUME_EXIT rc=$rc $(date -u -Is)"' EXIT
export WINVM_ROOT=/workspace/sweep-win-56766467
export WINVM_QEMU=/workspace/bench/kf3-bins/24906bf3/qemu-system-x86_64
W=scripts/bench/windows/win_vm.sh
[ "$(git rev-parse HEAD)" = 24906bf334665c9000b16b271005593daa3586c7 ]
[ ! -e /root/prov/sweep_windows580_accept.txt ]
echo "WINDOWS580_RESUME_START same cold boot; no installer, reboot or timeout change $(date -u -Is)"
python3 /root/sweep_windows_receipt.py sweep580_validate_resume
timeout 45 bash "$W" ssh '$u=(Get-CimInstance Win32_ComputerSystem).UserName; $l=@(Get-Process LogonUI -ErrorAction SilentlyContinue); $e=@(Get-Process explorer -ErrorAction SilentlyContinue); $d=@(Get-Process dwm -ErrorAction SilentlyContinue); $g=@(Get-CimInstance Win32_VideoController | Where-Object PNPDeviceID -like "PCI\VEN_10DE*"); if($u -ne "KFWIN\kf" -or $l.Count -or !$e.Count -or !$d.Count -or $g.Count -ne 1 -or $g[0].DriverVersion -ne "32.0.15.8088" -or $g[0].ConfigManagerErrorCode -ne 0) { throw "Console/GPU readiness guard refused" }; "DESKTOP_READY=1 user=$u explorer=$($e.Count) dwm=$($d.Count) LogonUI=$($l.Count) actual8088_Code0=1"' >/root/prov/sweep_windows580_desktop_ready.txt 2>&1
grep '^DESKTOP_READY=1 ' /root/prov/sweep_windows580_desktop_ready.txt
timeout 30 bash "$W" ssh 'New-Item -ItemType Directory -Force C:\ProgramData\KayfabeSweep | Out-Null'
for f in cup2.exe cup3.exe cup8.exe kf_dxprobe.exe accept580.ps1 hold580.ps1; do
  bash "$W" scp-to "/root/sweep-probes/$f" "C:/ProgramData/KayfabeSweep/$f"
done
timeout 900 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\ProgramData\KayfabeSweep\accept580.ps1' >/root/prov/sweep_windows580_accept.txt 2>&1
grep 'WIN_ACCEPT_EXIT rc=0 ' /root/prov/sweep_windows580_accept.txt
# cup2's inherited native probe returns0 even on a copy mismatch: grade values too.
grep -aF 'CE rv=0xabcd1234 want=0xabcd1234 -> PASS' /root/prov/sweep_windows580_accept.txt
grep -aF 'KERNEL rv=43 want=43 -> PASS' /root/prov/sweep_windows580_accept.txt
grep -aF 'CUP8 RESULT N=2048 bad=0 maxerr=0 ' /root/prov/sweep_windows580_accept.txt
[ "$(grep -ac '^KFDX RESULT OK' /root/prov/sweep_windows580_accept.txt)" = 2 ]
bash "$W" shot sweep580_accept
timeout 900 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\ProgramData\KayfabeSweep\hold580.ps1' >/root/prov/sweep_windows580_hold.txt 2>&1
grep 'HOLD_EXIT rc=0 ' /root/prov/sweep_windows580_hold.txt
bash "$W" shot sweep580_hold
bash "$W" check --kf3 >/root/prov/sweep_windows580_check.txt 2>&1
echo "WINDOWS580_SUITE_HOLD_COMPLETE $(date -u -Is)"
# Leave the running, verified VM available for one final screenshot/export.
# The controller will stop it before another stage.
