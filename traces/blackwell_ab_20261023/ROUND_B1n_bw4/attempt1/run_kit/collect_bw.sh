#!/bin/bash
# Generic-VGA collection boot of one arm's disk: NO kf3 device, NO GPU work, no kayfabe flag.
# Public metadata as before (events, WER, dump headers, clock offset, minidump hash), then the
# private capture (owner 2026-10-24 decision 2): guest dumps incl. LiveKernelReports subfolders
# and WER folders, the full event log (.evtx), and the arm's full kf3/QEMU logs, packed with a
# sha256 manifest under /root/capture/<run>/ for the operator to copy off before teardown.
# Bounded: guest work stops at COLLECT_BUDGET seconds after the QEMU start; the VM is always
# stopped. Also used once after a fatal stop (fatal_collect.py). Evidence steps never gate.
set -euo pipefail
. /root/sweep2/common.sh
run=${1:?arm name}; [[ $run =~ ^[a-z0-9_]{1,40}$ ]]
exec >"/root/prov/collect_${run}.log" 2>&1
COLLECT_BUDGET=600
launched=0; owned_pid=
cleanup(){
 rc=$?; trap - EXIT; set +e
 if [ "$launched" = 1 ] && [ -f "$VM/run/qemu.pid" ] &&
  [ "$(cat "$VM/run/qemu.pid")" = "$owned_pid" ]; then timeout 300 bash "$W" stop; fi
 for i in $(seq 1 60); do [ ! -f "$VM/run/qemu.pid" ] && break; sleep 1; done
 python3 -I /root/sweep2/capture_pack.py "$run" || echo CAPTURE_PACK_FAILED
 echo "COLLECT_EXIT run=$run rc=$rc $(date -u -Is)"; exit "$rc"
}
trap cleanup EXIT
cd /root/kayfabe
[ "$(git rev-parse HEAD)" = "$BASE" ]
[ -z "$(pgrep -x qemu-system-x86 || true)" ]
[ -f "$VM/disk/${run}.qcow2" ]
while IFS= read -r key; do unset "$key"; done < <(compgen -A variable KF3_)
export WINVM_QEMU=/workspace/bench/kf3-bins/${BASE:0:8}/qemu-system-x86_64
echo "COLLECT_START run=$run (standard VGA, no kf3 device) budget=${COLLECT_BUDGET}s" \
 "$(date -u -Is)"
bash "$W" run --overlay "$run" --no-kf3 --once --max-boots 1 --tag "${run}_collect" --detach
launched=1
start=$SECONDS
# step BOUND CMD...: run CMD with min(BOUND, remaining budget); skip once under 30 s remain.
step(){
 local bound=$1 left; shift
 left=$((COLLECT_BUDGET-(SECONDS-start)))
 if [ "$left" -lt 30 ]; then echo "COLLECT_DEADLINE_SKIP $*"; return 124; fi
 [ "$bound" -le "$left" ] || bound=$left
 timeout "$bound" "$@"
}
while [ ! -f "$VM/run/qemu.pid" ]; do [ $((SECONDS-start)) -lt 90 ]; sleep 1; done
owned_pid=$(cat "$VM/run/qemu.pid")
for i in $(seq 1 90); do
 [ -e "/proc/$owned_pid/exe" ] && [ $((SECONDS-start)) -lt 450 ]
 timeout 20 bash "$W" ssh 'echo KF_SSH_UP' </dev/null 2>/dev/null | grep -q KF_SSH_UP && break
 sleep 5
done
timeout 20 bash "$W" ssh 'echo KF_SSH_UP' </dev/null | grep KF_SSH_UP
guest_now=$(timeout 20 bash "$W" ssh "[DateTime]::UtcNow.ToString('o')" </dev/null | tr -d '\r')
echo "CLOCK host=$(date -u +%Y-%m-%dT%H:%M:%S.%N) guest=$guest_now"
bash "$W" scp-to /root/sweep2/collect_crash.ps1 C:/kf/collect_crash.ps1
ps='powershell -NoProfile -ExecutionPolicy Bypass -File'
step 180 bash "$W" ssh "$ps C:\\kf\\collect_crash.ps1" \
 </dev/null >"/root/prov/collect_${run}.txt" 2>&1 || echo "COLLECT_STEP_RC crash=$?"
grep '^COLLECT_DONE' "/root/prov/collect_${run}.txt" || echo COLLECT_STEP_INCOMPLETE crash
echo "COLLECT_METADATA_READY $(date -u -Is)"
bash "$W" scp-to /root/sweep2/wer_times.ps1 C:/kf/wer_times.ps1
step 120 bash "$W" ssh "$ps C:\\kf\\wer_times.ps1" \
 </dev/null >"/root/prov/wer_times_${run}.txt" 2>&1 || echo "COLLECT_STEP_RC wer=$?"
grep '^WER_TIMES_DONE' "/root/prov/wer_times_${run}.txt" || echo COLLECT_STEP_INCOMPLETE wer
bash "$W" scp-to /root/sweep2/dump_hdr_hex.ps1 C:/kf/dump_hdr_hex.ps1
step 180 bash "$W" ssh "$ps C:\\kf\\dump_hdr_hex.ps1" \
 </dev/null >"/root/prov/dump_hdr_${run}.txt" 2>&1 || echo "COLLECT_STEP_RC hdr=$?"
grep '^HDR_DONE' "/root/prov/dump_hdr_${run}.txt" || echo COLLECT_STEP_INCOMPLETE hdr
mkdir -p "/root/prov/arms/$run"
python3 -I /root/sweep2/dump_header.py --hexlines <"/root/prov/dump_hdr_${run}.txt" \
 >"/root/prov/arms/$run/dump_headers.jsonl" || echo COLLECT_STEP_INCOMPLETE dump_header
q="Get-CimInstance Win32_PnPSignedDriver | Where-Object {\$_.DeviceClass -eq 'DISPLAY'}"
q+=" | Select-Object DeviceName,DriverVersion,DriverDate,InfName | ConvertTo-Json -Compress"
step 120 bash "$W" ssh "$q" </dev/null >"/root/prov/arms/$run/display_driver.json" 2>&1 || true
# Small minidump(s), public hash only (as before).
mkdir -p "/root/prov/dumps/$run"
step 300 bash "$W" scp-from C:/Windows/Minidump "/root/prov/dumps/$run/" ||
 echo MINIDUMP_COPY_FAILED
find "/root/prov/dumps/$run" -type f -name '*.dmp' -exec sha256sum {} \; \
 >"/root/prov/arms/$run/minidump_sha256.txt" 2>/dev/null || true
# Private capture: stage in the guest, copy the staged folder to the box capture dir.
mkdir -p "/root/capture/$run"
cap_dir=/root/capture/$run
[ ! -e "$cap_dir/guest" ] || mv "$cap_dir/guest" "$cap_dir/guest.prev.$$"
bash "$W" scp-to /root/sweep2/capture_guest.ps1 C:/kf/capture_guest.ps1
step 300 bash "$W" ssh "$ps C:\\kf\\capture_guest.ps1" \
 </dev/null >"/root/prov/capture_guest_${run}.txt" 2>&1 || echo "COLLECT_STEP_RC capture=$?"
grep '^CAPTURE_GUEST_DONE' "/root/prov/capture_guest_${run}.txt" ||
 echo COLLECT_STEP_INCOMPLETE capture
step 420 bash "$W" scp-from C:/kf/cap "/root/capture/$run/guest" || echo CAPTURE_COPY_INCOMPLETE
echo "COLLECT_GUEST_SECONDS=$((SECONDS-start))"
echo "COLLECT_ALL_DONE $(date -u -Is)"
