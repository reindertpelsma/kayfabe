#!/bin/bash
# usage: arm.sh <base|cand> <on|off> <run-name>
set -euo pipefail
. /root/bwr/common.sh
prod=$1; profile=$2; run=$3
case $prod in base) BIN=7f22d18d; SRC=$BASE;; cand) BIN=aa8fdea9; SRC=$CAND;; fix*) BIN=$(cat /root/bwr/${prod}.bin); SRC=$(cat /root/bwr/${prod}.src);; *) exit 2;; esac
[[ $profile = on || $profile = off ]]
[[ $run =~ ^[a-z0-9_]+$ ]]
exec >"/root/prov/arm_${run}.log" 2>&1
trap 'rc=$?; echo "ARM_EXIT run=$run rc=$rc $(date -u -Is)"' EXIT
cd /root/kayfabe
[ "$(git rev-parse HEAD)" = $BASE ]
export WINVM_QEMU=/workspace/bench/kf3-bins/$BIN/qemu-system-x86_64
[ -x "$WINVM_QEMU" ]
[ -z "$(pgrep -x qemu-system-x86 || true)" ]
[ ! -e "/root/prov/arm_${run}.started" ]
grep -q '^BIND_STOPPED_READY' /root/prov/bwr_bind.log
date -u -Is >"/root/prov/arm_${run}.started"
parent=$VM/disk/bw580_bind.qcow2; child=$VM/disk/${run}.qcow2
[ -f "$parent" ] && [ ! -e "$child" ]
qemu-img create -q -f qcow2 -F qcow2 -b "$parent" "$child"
while IFS= read -r key; do unset "$key"; done < <(compgen -A variable KF3_)
while IFS= read -r flag; do export "$flag"; done </root/bwr/p19.flags
export KF3_RPC_TRACE=1 KF3_RPC_BODY_TRACE=1
for kv in ${ARM_EXTRA:-}; do [[ $kv =~ ^KF3_[A-Z0-9_]+=[0-9A-Za-z_./-]+$ ]] && export "$kv" && echo "ARM_EXTRA $kv"; done
if [ "${ARM_TRACE:-0}" = 1 ]; then export WINVM_TRACE=1 KF3_BAR0_READ_TRACE=1; echo "ARM_TRACE diagnostic BAR0 trace ON (display reads + all writes)"; fi
echo "ARM_START run=$run prod=$prod source=$SRC bin=$BIN profile=$profile $(date -u -Is)"
extra=()
[ "$profile" = on ] || extra=(--kf3-extra display=off,gop=off,gop-efi=)
bash "$W" run --overlay "$run" --arm B --guest-driver 580.65.06 --once --max-boots 1 --tag "$run" "${extra[@]}" --detach
for i in $(seq 1 90); do timeout 20 bash "$W" ssh 'echo KF_SSH_UP' </dev/null 2>/dev/null | grep -q KF_SSH_UP && break; sleep 5; done
timeout 20 bash "$W" ssh 'echo KF_SSH_UP' </dev/null | grep KF_SSH_UP
python3 /root/bwr/receipt.py "arm_${run}" "$WINVM_QEMU" "$profile"
for f in verify_stage580.ps1 enable580_once.ps1 diag580.ps1; do bash "$W" scp-to "/root/bwr/probes/$f" "C:/kf/$f"; done
bash "$W" scp-to /root/bwr/events.ps1 C:/kf/events.ps1
timeout 120 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\verify_stage580.ps1' </dev/null >"/root/prov/arm_${run}_pre_enable.txt" 2>&1
grep '^STAGE580_READY=1 ' "/root/prov/arm_${run}_pre_enable.txt"
QLOG=$VM/logs/${run}_b1_qemu.log
if grep -aq 'kf-rm: rpc-trace' "$QLOG"; then echo PRECONDITION_REFUSED_GSP_WHILE_DISABLED; exit 7; fi
timeout 600 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\enable580_once.ps1' </dev/null >"/root/prov/arm_${run}_enable.txt" 2>&1 || true
cat "/root/prov/arm_${run}_enable.txt"
sleep 30
timeout 120 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\diag580.ps1' </dev/null >"/root/prov/arm_${run}_diag.txt" 2>&1 || true
cat "/root/prov/arm_${run}_diag.txt"
timeout 120 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\events.ps1' </dev/null >"/root/prov/arm_${run}_events.txt" 2>&1 || true
bash "$W" shot "$run" || true
python3 scripts/bench/windows/abort_point.py points "$QLOG" >"/root/prov/arm_${run}_abort_point.txt" 2>&1 || true
python3 -I /root/bwr/export.py "$QLOG" "/root/prov/arms/$run" || true
[ "${ARM_TRACE:-0}" = 1 ] && python3 -I /root/bwr/trace_window.py "$QLOG" "$VM/logs/${run}_b1_trace.log" "/root/prov/arm_${run}_trace_window.txt" || true
echo "ARM_DIAGNOSTIC_READY run=$run $(date -u -Is)"
timeout 300 bash "$W" stop || true
sleep 3
pgrep -x qemu-system-x86 && echo QEMU_STILL_RUNNING || echo QEMU_STOPPED
