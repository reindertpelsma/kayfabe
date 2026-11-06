#!/bin/bash
# observe.sh OUT: 10 Hz, one line per CHANGE of (qemu.pid content, its liveness, qemu/swtpm/run-loop pids)
VM=${WINVM_ROOT:-/workspace/winvm}/kfwin
out=$1 prev=""
while :; do
  pid=$(cat "$VM/run/qemu.pid" 2>/dev/null)
  alive=0
  if [ -n "$pid" ] && [ -d "/proc/$pid" ]; then alive=1; fi
  line="pidfile=${pid:-none} alive=$alive qemu=$(pgrep -x qemu-system-x86 | tr '\n' ,) swtpm=$(pgrep -x swtpm | tr '\n' ,) runloops=$(pgrep -f 'win_vm\.sh run' | tr '\n' ,)"
  if [ "$line" != "$prev" ]; then echo "$EPOCHREALTIME $(date -u +%T.%N | cut -c1-12) $line" >> "$out"; prev=$line; fi
  sleep 0.1
done
