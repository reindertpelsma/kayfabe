#!/bin/bash
# Rental-only generic-VGA recovery and bounded public-symbol analysis.
set -euo pipefail
. /root/sweep2/common.sh
run=${1:?arm name}; [[ $run =~ ^[a-z0-9_]{1,40}$ ]]
exec >"/root/prov/collect_${run}.log" 2>&1
launched=0; owned_pid=; proxy_pid=; tunnel_pid=
cleanup(){
 rc=$?; trap - EXIT; set +e
 for p in "$tunnel_pid" "$proxy_pid"; do [ -z "$p" ] || kill -TERM "$p" 2>/dev/null; done
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
bash "$W" scp-to /root/sweep2/collect_crash.ps1 C:/kf/collect_crash.ps1
timeout 180 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\collect_crash.ps1' </dev/null >"/root/prov/collect_${run}.txt" 2>&1
grep '^COLLECT_DONE' "/root/prov/collect_${run}.txt"
echo "COLLECT_METADATA_READY $(date -u -Is)"
if grep -q '^FILE .*\.dmp ' "/root/prov/collect_${run}.txt"; then
 # Old 7z rejected the outer ZIP64 headers; the same downloaded bundle was
 # successfully CRC-checked by Python. Keep both extraction receipts.
 grep -q '^PUBLIC_DEBUGGER_EXTRACT_EXIT rc=0 ' /root/prov/public_debugger_extract.log
 grep -q '^PUBLIC_DEBUGGER_EXTRACT_READY ' /root/prov/public_debugger_extract.ready
 [ -f /root/sweep2/windbg_pkg/x64/amd64/cdb.exe ]
 timeout 30 bash "$W" ssh 'powershell -NoProfile -Command "New-Item -ItemType Directory -Force C:\kf\windbg | Out-Null"' </dev/null
 timeout 300 bash "$W" scp-to /root/sweep2/windbg_pkg/x64/amd64 C:/kf/windbg/
 bash "$W" scp-to /root/sweep2/analyze_dumps.ps1 C:/kf/analyze_dumps.ps1
 timeout 900 python3 -I /root/sweep2/symbol_proxy.py >"/root/prov/symbol_proxy_${run}.log" 2>&1 &
 proxy_pid=$!
 timeout 900 ssh -n -N -p 2224 -i "$VM/secrets/ssh_ed25519" -o StrictHostKeyChecking=accept-new -o "UserKnownHostsFile=$VM/run/known_hosts" -o ConnectTimeout=8 -o ServerAliveInterval=15 -o ServerAliveCountMax=4 -o BatchMode=yes -o LogLevel=ERROR -o ExitOnForwardFailure=yes -R 127.0.0.1:18765:127.0.0.1:18766 kf@127.0.0.1 >"/root/prov/symbol_tunnel_${run}.log" 2>&1 &
 tunnel_pid=$!
 sleep 2
 kill -0 "$proxy_pid" && kill -0 "$tunnel_pid"
 echo "PUBLIC_ANALYSIS_START $(date -u -Is)"
 timeout 600 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\analyze_dumps.ps1' </dev/null >"/root/prov/public_analysis_${run}.txt" 2>&1
 grep '^PUBLIC_DUMP_ANALYSIS_DONE' "/root/prov/public_analysis_${run}.txt"
 echo "PUBLIC_ANALYSIS_DONE $(date -u -Is)"
else echo NO_DUMP_FILES_REPORTED; fi
