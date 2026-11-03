#!/bin/bash
# Generic-VGA recovery boot after an arm: events, WER, dump headers (public metadata), clock offset, minidump copy.
set -euo pipefail
. /root/sweep2/common.sh
run=${1:?arm name}; [[ $run =~ ^[a-z0-9_]{1,40}$ ]]
exec >"/root/prov/collect_${run}.log" 2>&1
launched=0; owned_pid=
cleanup(){
 rc=$?; trap - EXIT; set +e
 if [ "$launched" = 1 ] && [ -f "$VM/run/qemu.pid" ] && [ "$(cat "$VM/run/qemu.pid")" = "$owned_pid" ]; then timeout 300 bash "$W" stop; fi
 echo "COLLECT_EXIT run=$run rc=$rc $(date -u -Is)"; exit "$rc"
}
trap cleanup EXIT
cd /root/kayfabe
[ "$(git rev-parse HEAD)" = "$BASE" ]
[ -z "$(pgrep -x qemu-system-x86 || true)" ]
[ -f "$VM/disk/${run}.qcow2" ]
while IFS= read -r key; do unset "$key"; done < <(compgen -A variable KF3_)
export WINVM_QEMU=/workspace/bench/kf3-bins/${BASE:0:8}/qemu-system-x86_64
echo "COLLECT_START run=$run (standard VGA, no kf3 device) $(date -u -Is)"
bash "$W" run --overlay "$run" --no-kf3 --once --max-boots 1 --tag "${run}_collect" --detach
launched=1
start=$SECONDS
while [ ! -f "$VM/run/qemu.pid" ]; do [ $((SECONDS-start)) -lt 90 ]; sleep 1; done
owned_pid=$(cat "$VM/run/qemu.pid")
for i in $(seq 1 90); do
 [ -e "/proc/$owned_pid/exe" ] && [ $((SECONDS-start)) -lt 450 ]
 timeout 20 bash "$W" ssh 'echo KF_SSH_UP' </dev/null 2>/dev/null | grep -q KF_SSH_UP && break
 sleep 5
done
timeout 20 bash "$W" ssh 'echo KF_SSH_UP' </dev/null | grep KF_SSH_UP
echo "CLOCK host=$(date -u +%Y-%m-%dT%H:%M:%S.%N) guest=$(timeout 20 bash "$W" ssh "[DateTime]::UtcNow.ToString('o')" </dev/null | tr -d '\r')"
bash "$W" scp-to /root/sweep2/collect_crash.ps1 C:/kf/collect_crash.ps1
timeout 180 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\collect_crash.ps1' </dev/null >"/root/prov/collect_${run}.txt" 2>&1
grep '^COLLECT_DONE' "/root/prov/collect_${run}.txt"
echo "COLLECT_METADATA_READY $(date -u -Is)"
bash "$W" scp-to /root/sweep2/wer_times.ps1 C:/kf/wer_times.ps1
timeout 120 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\wer_times.ps1' </dev/null >"/root/prov/wer_times_${run}.txt" 2>&1
grep '^WER_TIMES_DONE' "/root/prov/wer_times_${run}.txt"
bash "$W" scp-to /root/sweep2/dump_hdr_hex.ps1 C:/kf/dump_hdr_hex.ps1
timeout 180 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\dump_hdr_hex.ps1' </dev/null >"/root/prov/dump_hdr_${run}.txt" 2>&1
grep '^HDR_DONE' "/root/prov/dump_hdr_${run}.txt"
mkdir -p "/root/prov/arms/$run"
python3 -I /root/sweep2/dump_header.py --hexlines <"/root/prov/dump_hdr_${run}.txt" >"/root/prov/arms/$run/dump_headers.jsonl"
timeout 120 bash "$W" ssh "Get-CimInstance Win32_PnPSignedDriver | Where-Object {\$_.DeviceClass -eq 'DISPLAY'} | Select-Object DeviceName,DriverVersion,DriverDate,InfName | ConvertTo-Json -Compress" </dev/null >"/root/prov/arms/$run/display_driver.json" 2>&1 || true
# Small minidump(s) only (data); the live dump and every executable stay on the rental.
mkdir -p "/root/prov/dumps/$run"
timeout 300 bash "$W" scp-from C:/Windows/Minidump "/root/prov/dumps/$run/" || echo MINIDUMP_COPY_FAILED
find "/root/prov/dumps/$run" -type f -name '*.dmp' -exec sha256sum {} \; >"/root/prov/arms/$run/minidump_sha256.txt" 2>/dev/null || true
echo "COLLECT_ALL_DONE $(date -u -Is)"
