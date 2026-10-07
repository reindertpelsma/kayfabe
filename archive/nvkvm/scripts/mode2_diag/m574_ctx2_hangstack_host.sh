#!/usr/bin/env bash
# m574_ctx2_hangstack_host.sh — ON HOST. cont.28: find the REAL CTX2 cuCtxCreate hang
# point (the cwfp(84) framing only fires at teardown). Fresh boot (no m2trace), then
# run cupctx2_hangstack_guest.sh which backgrounds cupctx2_min and samples its kernel
# stack / wchan / syscall over ~110s to catch where CTX2 create spins.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
D=/workspace/nvkvm

echo "==> fresh restart VM (default mode, no m2trace)"
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
nohup bash $D/scripts/run_mode2_vm.sh >/tmp/m574_launch.log 2>&1 &
sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; tail -8 /tmp/m574_launch.log; exit 1; }

echo "==> wait guest ssh"
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; echo "  up ${i}x5s"; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; tail -8 /tmp/m574_launch.log; exit 1; }

echo "==> stage cupctx2_min.c"
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  $D/tests/mode2/cupctx2_min.c ubuntu@localhost:/tmp/cupctx2_min.c 2>/dev/null && echo "  staged"

echo "==> run hang-stack sampler (SAMPLES=${SAMPLES:-14} x 8s)"
$SSHG "SAMPLES=${SAMPLES:-14} bash -s" < $D/scripts/mode2_diag/cupctx2_hangstack_guest.sh 2>&1
echo "============ END (look for the repeated kernel stack = the hang) ============"
