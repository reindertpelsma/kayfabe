#!/usr/bin/env bash
set -euo pipefail
export PATH=/root/.cargo/bin:$PATH
cd /root/kayfabe
exec >/root/prov/sweep_windows580_validate.log 2>&1
trap 'rc=$?; echo "WINDOWS580_VALIDATE_EXIT rc=$rc $(date -u -Is)"' EXIT
[ "$(git rev-parse HEAD)" = 24906bf334665c9000b16b271005593daa3586c7 ]
export WINVM_ROOT=/workspace/sweep-win-56766467
export WINVM_QEMU=/workspace/bench/kf3-bins/24906bf3/qemu-system-x86_64
export WINVM_RAM_MB=8192 WINVM_SMP=8
W=scripts/bench/windows/win_vm.sh
[ -z "$(pgrep -x qemu-system-x86 || true)" ]
while IFS= read -r key; do unset "$key"; done < <(compgen -A variable KF3_)
while IFS= read -r flag; do export "$flag"; done </root/sweep-p19.flags
export KF3_RPC_TRACE=1 KF3_RPC_BODY_TRACE=1
echo "H_WIN580_VALIDATE_START source=$(git rev-parse HEAD) $(date -u -Is)"
echo 'FALSIFIER wrong580.88/Code!=0/noDWMExplorer/CUDA/D3D/blank/TDR4101 at>=420s'
bash "$W" run --overlay sweep580 --arm B --guest-driver 580.65.06 --once --max-boots 1 --tag sweep580_validate --detach
for i in $(seq 1 90); do
  if timeout 20 bash "$W" ssh 'echo KF_SSH_UP' 2>/dev/null | grep -q KF_SSH_UP; then break; fi
  sleep 5
done
timeout 20 bash "$W" ssh 'echo KF_SSH_UP' | grep KF_SSH_UP
python3 /root/sweep_windows_receipt.py sweep580_validate
python3 /root/sweep_signin_kf.py "$WINVM_ROOT/kfwin/run/qmp.sock" "$WINVM_ROOT/kfwin/run/qga.sock" "$WINVM_ROOT/kfwin/secrets/win_password"
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
