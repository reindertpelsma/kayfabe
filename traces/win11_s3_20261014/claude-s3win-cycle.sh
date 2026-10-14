#!/bin/bash
# cycle.sh <tag> [perf]  — suspend run 2990 via SetSuspendState, optionally record pio/mmio across the wake
T=$1; P=${2:-}
R=/var/lib/kf-windows-20261005/boundary-kayfabe-2990
Q=$(sed -n 's/^QPID=//p' /var/lib/kf-windows-20261005/windows-broker-run2990.state)
python3 /data/claude-s3win-qmp.py status
wc -l < $R/qemu.log > /data/claude-s3win-line.$T.pre
python3 /data/kfagent_server.py run "Start-Process cmd.exe -ArgumentList \"/c timeout /t 4 /nobreak & rundll32.exe powrprof.dll,SetSuspendState 0,1,0\" -WindowStyle Hidden; \"sched \" + (Get-Date).ToString(\"o\")" 20
for i in $(seq 90); do s=$(python3 /data/claude-s3win-qmp.py status); [ "$s" = suspended ] && break; sleep 1; done
echo "status=$s at $(date -Is)"
[ "$s" = suspended ] || exit 1
wc -l < $R/qemu.log > /data/claude-s3win-line.$T.wake
if [ -n "$P" ]; then
  perf record -q -e kvm:kvm_pio,kvm:kvm_mmio -p $Q -o /tmp/claude-s3win-perf.$T.data -- sleep 25 >/dev/null 2>&1 &
  PP=$!
  sleep 1
fi
echo "WAKE at $(date -Is)"
python3 /data/claude-s3win-qmp.py wakeup
[ -n "$P" ] && wait $PP
echo "done $(date -Is)"
