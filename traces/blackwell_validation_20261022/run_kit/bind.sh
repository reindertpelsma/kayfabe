#!/usr/bin/env bash
set -euo pipefail
. /root/sweep2/common.sh
cd /root/kayfabe
exec >/root/prov/bwr_bind.log 2>&1
trap 'rc=$?; echo "BIND_EXIT rc=$rc $(date -u -Is)"' EXIT
[ "$(git rev-parse HEAD)" = $BASE ]
export WINVM_QEMU=/workspace/bench/kf3-bins/8f3a9bb3/qemu-system-x86_64 CARGO_BUILD_JOBS=16
[ -f "$VM/installed" ]
[ -z "$(pgrep -x qemu-system-x86 || true)" ]
while IFS= read -r key; do unset "$key"; done < <(compgen -A variable KF3_)
while IFS= read -r flag; do export "$flag"; done </root/sweep2/p19.flags
export KF3_RPC_TRACE=1 KF3_RPC_BODY_TRACE=1
echo "BIND_START source=$(git rev-parse HEAD) $(date -u -Is) (bind only, no enable; not a measured init arm)"
bash "$W" sign
bash "$W" run --overlay bw580_bind --arm B --guest-driver 580.65.06 --once --max-boots 1 --tag bw580_bind --detach
for i in $(seq 1 90); do timeout 20 bash "$W" ssh 'echo KF_SSH_UP' 2>/dev/null | grep -q KF_SSH_UP && break; sleep 5; done
timeout 20 bash "$W" ssh 'echo KF_SSH_UP' | grep KF_SSH_UP
python3 /root/sweep2/bind_receipt.py bwr_bind "$WINVM_QEMU" bind
bash "$W" nv-install --no-enable
timeout 60 bash "$W" ssh 'Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion,ConfigManagerErrorCode,PNPDeviceID | ConvertTo-Json -Compress' > /root/prov/bwr_binding.json
cat /root/prov/bwr_binding.json
bash "$W" scp-to /root/sweep2/probes/prepare_cold580.ps1 C:/kf/prepare_cold580.ps1
timeout 120 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\prepare_cold580.ps1' >/root/prov/bwr_cold_ready.txt 2>&1
cat /root/prov/bwr_cold_ready.txt
grep -q 'COLD580_READY=1' /root/prov/bwr_cold_ready.txt
bash "$W" scp-to /root/sweep2/crashdump.ps1 C:/kf/crashdump.ps1
timeout 120 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\crashdump.ps1' >/root/prov/bwr_crashdump.txt 2>&1
cat /root/prov/bwr_crashdump.txt
grep -q 'CRASHDUMP_READY=1' /root/prov/bwr_crashdump.txt
timeout 300 bash "$W" stop
python3 -E -s /root/sweep2/guard/blackwell/shutdown_census.py /root/prov/bwr_bind_process_receipt.json /root/prov/native_receipt.json.accepted.json /root/prov/bwr_shutdown_census.json
chmod 0400 "$VM/disk/bw580_bind.qcow2"
sha256sum "$VM/disk/bw580_bind.qcow2" > /root/prov/bwr_parent_sha256.txt
echo "BIND_STOPPED_READY $(date -u -Is)"
