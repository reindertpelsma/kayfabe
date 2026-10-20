#!/usr/bin/env bash
set -euo pipefail
export PATH=/root/.cargo/bin:$PATH
cd /root/kayfabe
exec >/root/prov/sweep_windows580.log 2>&1
trap 'rc=$?; echo "WINDOWS580_EXIT rc=$rc $(date -u -Is)"' EXIT
[ "$(git rev-parse HEAD)" = 0c87acda802b18a610739e75f2b50049a9247067 ]
export WINVM_ROOT=/workspace/sweep-win-56816419
export WINVM_QEMU=/workspace/bench/kf3-bins/0c87acda/qemu-system-x86_64
export WINVM_RAM_MB=8192 WINVM_SMP=8 CARGO_BUILD_JOBS=8
VM=$WINVM_ROOT/kfwin
W=scripts/bench/windows/win_vm.sh
[ -f "$VM/installed" ]
[ -z "$(pgrep -x qemu-system-x86 || true)" ]
while IFS= read -r key; do unset "$key"; done < <(compgen -A variable KF3_)
while IFS= read -r flag; do export "$flag"; done </root/sweep-p19.flags
export KF3_RPC_TRACE=1 KF3_RPC_BODY_TRACE=1
echo 'H_WIN580: falsified by wrong driver, Code!=0, missing desktop, CUDA/D3D failure, blank image, or System4101 during >=421s hold.'
echo "WINDOWS580_START source=$(git rev-parse HEAD) $(date -u -Is)"
cat /root/sweep-p19.flags
# Compile/sign before starting a timed workload; reuse the original TPM and vars.
bash "$W" sign
bash "$W" run --overlay sweep580 --arm B --guest-driver 580.65.06 --once --max-boots 1 --tag sweep580_bind --detach
for i in $(seq 1 90); do
  if timeout 20 bash "$W" ssh 'echo KF_SSH_UP' 2>/dev/null | grep -q KF_SSH_UP; then break; fi
  sleep 5
done
timeout 20 bash "$W" ssh 'echo KF_SSH_UP' | grep KF_SSH_UP
python3 /root/sweep_windows_receipt.py sweep580_bind
bash "$W" nv-install
# Export actual binding, without accepting the installer command's status as proof.
timeout 60 bash "$W" ssh 'Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion,ConfigManagerErrorCode,PNPDeviceID | ConvertTo-Json -Compress' > /root/prov/sweep_windows580_binding.json
cat /root/prov/sweep_windows580_binding.json
bash "$W" scp-to /root/sweep-probes/prepare_cold580.ps1 C:/kf/prepare_cold580.ps1
timeout 120 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\prepare_cold580.ps1' >/root/prov/sweep_windows580_cold_ready.txt 2>&1
cat /root/prov/sweep_windows580_cold_ready.txt
timeout 300 bash "$W" stop
echo "WINDOWS580_BIND_STOPPED $(date -u -Is)"
