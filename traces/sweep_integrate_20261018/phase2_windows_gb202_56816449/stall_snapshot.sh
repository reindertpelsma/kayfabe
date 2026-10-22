#!/bin/bash
set -euo pipefail
profile=${1:?off_no_rom or on}
[[ "$profile" = off_no_rom || "$profile" = on ]]
VM=/workspace/sweep-win-56816449/kfwin
D=/root/prov/bw580_${profile}_stall
mkdir "$D"
exec >"$D/receipt.txt" 2>&1
trap 'rc=$?; echo "STALL_SNAPSHOT_EXIT rc=$rc $(date -u -Is)"' EXIT
echo "STALL_SNAPSHOT_START profile=$profile $(date -u -Is)"
pid=$(cat "$VM/run/qemu.pid")
[ "$(readlink /proc/$pid/exe)" = /workspace/bench/kf3-bins/0c87acda/qemu-system-x86_64 ]
echo "owned_qemu_pid=$pid source=$(git -C /root/kayfabe rev-parse HEAD)"
ps -L -p "$pid" -o pid,tid,pcpu,stat,wchan:32,comm >"$D/threads.txt"
timeout 20 nvidia-smi --query-gpu=name,driver_version,utilization.gpu,utilization.memory --format=csv,noheader >"$D/host_smi.txt" 2>&1 || echo 'HOST_SMI_QUERY_UNAVAILABLE'
python3 /root/kayfabe/scripts/bench/windows/qmp.py "$VM/run/qmp.sock" cmd query-status >"$D/qmp_status.json" 2>&1 || echo QMP_STATUS_UNAVAILABLE
# Only our public-source QEMU is inspected; no Windows binary is examined.
if command -v gdb >/dev/null; then
 timeout 25 gdb -q -n -batch -ex 'set pagination off' -ex 'set print frame-arguments none' -ex "attach $pid" -ex 'thread apply all bt 12' -ex detach -ex quit >"$D/gdb_rental_only.txt" 2>&1 || echo GDB_SNAPSHOT_INCOMPLETE
fi
python3 -B /root/sweep_time_sampler.py "$VM/logs/bw580_${profile}_b1_qemu.log" "$D/metadata.jsonl" --seconds 1 --interval 1
sha256sum "$VM/logs/bw580_${profile}_b1_qemu.log" >"$D/qemu_sha256.txt"
echo "STALL_SNAPSHOT_COMPLETE $(date -u -Is)"
