#!/usr/bin/env bash
set -u
PORT=2223; SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 ubuntu@localhost"
SCPG="scp -q -P $PORT -o StrictHostKeyChecking=no"
$SSHG 'sync;sync' 2>/dev/null||true; pkill -TERM qemu-system 2>/dev/null; sleep 3; pkill -9 qemu-system 2>/dev/null; sleep 2
NVKVM_FRESH=0 NVKVM_M2FWD=on nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 & sleep 3
up=0; for i in $(seq 1 50); do if $SSHG echo OK 2>/dev/null|grep -q OK; then up=1; break; fi; sleep 5; done
[ "$up" = 1 ]||{ echo NO_BOOT; tail -5 /tmp/m0_launch.log; exit 1; }
echo BOOTED
$SSHG 'mkdir -p /tmp/mode2_diag'
for t in 1 2 3; do $SCPG /workspace/nvkvm/tests/mode2/cup2.c ubuntu@localhost:/tmp/cup2.c 2>/dev/null && break; sleep 3; done
$SCPG /tmp/mode2_diag/nvtrace.c ubuntu@localhost:/tmp/mode2_diag/nvtrace.c 2>/dev/null
$SCPG /tmp/cup2_nvtrace_inner.sh ubuntu@localhost:/tmp/cup2_nvtrace_inner.sh 2>/dev/null
timeout 200 $SSHG 'bash /tmp/cup2_nvtrace_inner.sh' 2>&1 | tail -6
$SCPG ubuntu@localhost:/tmp/guest_nvtrace.txt /tmp/guest_nvtrace.txt 2>/dev/null
echo "pulled: $(wc -l < /tmp/guest_nvtrace.txt 2>/dev/null) lines"
$SSHG 'sync;sync' 2>/dev/null||true; pkill -9 qemu-system 2>/dev/null
