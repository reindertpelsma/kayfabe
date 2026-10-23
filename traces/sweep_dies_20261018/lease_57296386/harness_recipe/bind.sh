#!/usr/bin/env bash
set -euo pipefail
. /root/s2/common.sh
cd /root/kayfabe
. scripts/bench/windows/readiness.sh
exec >/root/prov/bwr_bind.log 2>&1
trap 'rc=$?; echo "BIND_EXIT rc=$rc $(date -u -Is)"' EXIT
[ "$(git rev-parse HEAD)" = $BASE ]
export WINVM_QEMU=/workspace/bench/kf3-bins/${BASE:0:8}/qemu-system-x86_64 CARGO_BUILD_JOBS=16
[ -f "$VM/installed" ]
python3 /root/kayfabe/scripts/bench/diesweep/desktop/live_qemu.py --assert-none
while IFS= read -r key; do unset "$key"; done < <(compgen -A variable KF3_)
while IFS= read -r flag; do export "$flag"; done </root/s2/p19.flags
export KF3_RPC_TRACE=1 KF3_RPC_BODY_TRACE=1
echo "BIND_START source=$(git rev-parse HEAD) $(date -u -Is) (bind only, no enable; not a measured init arm)"
bash "$W" sign
readiness_stage /root/prov/bwr_bind_readiness_stage.json 600 3600
bash "$W" run --overlay bw580_bind --arm B --guest-driver 580.65.06 --once --max-boots 1 --tag bw580_bind --detach
readiness_wait bwr_bind_ssh bash "$W" ssh 'echo KF_SSH_UP'
python3 /root/s2/bind_receipt.py bwr_bind "$WINVM_QEMU" bind
readiness_step once bwr_install 300 1200 bash "$W" nv-install --no-enable
readiness_step query bwr_cim 30 240 bash "$W" ssh 'Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion,ConfigManagerErrorCode,PNPDeviceID | ConvertTo-Json -Compress' > /root/prov/bwr_binding.json
cat /root/prov/bwr_binding.json
bash "$W" scp-to /root/s2/probes/prepare_cold580.ps1 C:/kf/prepare_cold580.ps1
readiness_step once bwr_cold 60 480 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\prepare_cold580.ps1' >/root/prov/bwr_cold_ready.txt 2>&1
cat /root/prov/bwr_cold_ready.txt
grep -q 'COLD580_READY=1' /root/prov/bwr_cold_ready.txt
bash "$W" scp-to /root/s2/crashdump.ps1 C:/kf/crashdump.ps1
readiness_step once bwr_crashdump 60 480 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\crashdump.ps1' >/root/prov/bwr_crashdump.txt 2>&1
cat /root/prov/bwr_crashdump.txt
grep -q 'CRASHDUMP_READY=1' /root/prov/bwr_crashdump.txt
readiness_step once bwr_stop 120 600 bash "$W" stop
python3 /root/kayfabe/scripts/bench/diesweep/desktop/live_qemu.py --assert-none
chmod 0400 "$VM/disk/bw580_bind.qcow2"
sha256sum "$VM/disk/bw580_bind.qcow2" > /root/prov/bwr_parent_sha256.txt
echo "BIND_STOPPED_READY $(date -u -Is)"
