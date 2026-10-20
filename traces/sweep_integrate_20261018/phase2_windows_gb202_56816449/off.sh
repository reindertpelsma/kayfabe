#!/bin/bash
set -euo pipefail
profile=off
run=off_no_rom
exec >"/root/prov/bw580_${run}.log" 2>&1
trap 'rc=$?; echo "BW580_ARM_EXIT profile=$profile rc=$rc $(date -u -Is)"' EXIT
export PATH=/root/.cargo/bin:$PATH
cd /root/kayfabe
[ "$(git rev-parse HEAD)" = 0c87acda802b18a610739e75f2b50049a9247067 ]
export WINVM_ROOT=/workspace/sweep-win-56816449 WINVM_QEMU=/workspace/bench/kf3-bins/0c87acda/qemu-system-x86_64 WINVM_RAM_MB=8192 WINVM_SMP=8
VM=$WINVM_ROOT/kfwin
W=scripts/bench/windows/win_vm.sh
[ -z "$(pgrep -x qemu-system-x86 || true)" ]
[ ! -e "/root/prov/bw580_${run}.started" ]
grep '^WINDOWS580_EXIT rc=0 ' /root/prov/sweep_windows580.log
grep 'COLD580_READY=1' /root/prov/sweep_windows580_cold_ready.txt
date -u -Is >"/root/prov/bw580_${run}.started"
parent=$VM/disk/bw580_bind.qcow2
child=$VM/disk/bw580_${run}.qcow2
[ -f "$parent" ] && [ ! -e "$child" ]
chmod 0400 "$parent"
qemu-img create -q -f qcow2 -F qcow2 -b "$parent" "$child"
qemu-img info --backing-chain --output=json "$child" >"/root/prov/bw580_${run}_disk_chain.json"
while IFS= read -r key; do unset "$key"; done < <(compgen -A variable KF3_)
while IFS= read -r flag; do export "$flag"; done </root/sweep-p19.flags
export KF3_RPC_TRACE=1 KF3_RPC_BODY_TRACE=1
echo "BW580_ARM_START profile=$profile source=$(git rev-parse HEAD) $(date -u -Is)"
echo 'H_B1 falsifier: display-off still Code43 without CA70/NV0073/CA7A; exact teardown path and explicit fn47 must be measured. No reply guesses.'
extra=()
[ "$profile" = on ] || extra=(--kf3-extra display=off,gop=off,gop-efi=)
bash "$W" run --overlay "bw580_${run}" --arm B --guest-driver 580.65.06 --once --max-boots 1 --tag "bw580_${run}" "${extra[@]}" --detach
python3 -B /root/sweep_time_sampler.py "$VM/logs/bw580_${run}_b1_qemu.log" "/root/prov/bw580_${run}_timeline.jsonl" --seconds 900 --interval 10 >"/root/prov/bw580_${run}_sampler.log" 2>&1 &
echo $! >"/root/prov/bw580_${run}_sampler.pid"
for i in $(seq 1 90); do
 if timeout 20 bash "$W" ssh 'echo KF_SSH_UP' </dev/null 2>/dev/null | grep -q KF_SSH_UP; then break; fi
 sleep 5
done
timeout 20 bash "$W" ssh 'echo KF_SSH_UP' </dev/null | grep KF_SSH_UP
python3 /root/sweep_bw_receipt.py "bw580_${run}" "$profile"
python3 /root/sweep_off_qom.py
for f in verify_stage580.ps1 enable580_once.ps1 diag580.ps1; do bash "$W" scp-to "/root/sweep-probes/$f" "C:/kf/$f"; done
timeout 120 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\verify_stage580.ps1' </dev/null >"/root/prov/bw580_${run}_pre_enable.txt" 2>&1
grep '^STAGE580_READY=1 ' "/root/prov/bw580_${run}_pre_enable.txt"
if grep -aq 'kf-rm: rpc-trace' "$VM/logs/bw580_${run}_b1_qemu.log"; then echo PRECONDITION_REFUSED_UNEXPECTED_GSP_WHILE_DISABLED; exit 7; fi
timeout 600 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\enable580_once.ps1' </dev/null >"/root/prov/bw580_${run}_enable.txt" 2>&1
cat "/root/prov/bw580_${run}_enable.txt"
timeout 120 bash "$W" ssh 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\diag580.ps1' </dev/null >"/root/prov/bw580_${run}_diag.txt" 2>&1
cat "/root/prov/bw580_${run}_diag.txt"
bash "$W" shot "bw580_${run}" || true
python3 scripts/bench/windows/abort_point.py points "$VM/logs/bw580_${run}_b1_qemu.log" >"/root/prov/bw580_${run}_abort_detector.txt"
echo "BW580_DIAGNOSTIC_READY profile=$profile $(date -u -Is)"
# Leave this owned VM running for bounded text screening or a Code0 suite.
# The controller stops it before another arm; do not auto-loop.
