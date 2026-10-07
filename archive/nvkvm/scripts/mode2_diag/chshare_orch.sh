#!/usr/bin/env bash
# chshare_orch.sh — ON HOST (vh). Fresh QEMU (m2cefwd, NO m2trace -> forge stays gated off),
# run chshare (channel backing isolation + sharing test). Captures any FB-phys collision in the
# emulated-FB backing (the #12 mechanism, isolated from the full scrubber-teardown path).
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 ubuntu@localhost"

echo "==> [1/5] copy deploy src -> build tree + rebuild QEMU (incremental)"
cp /workspace/nvkvm/src/qemu/nvkvm_gpu_emul.c /opt/qemu-src/hw/misc/nvkvm_gpu_emul.c
( cd /opt/qemu-src/build && ninja qemu-system-x86_64 && ninja install ) 2>&1 | tail -1 || { echo BUILD_FAIL; exit 1; }

echo "==> [2/5] fresh QEMU (m2cefwd=on, no m2trace)"
$SSHG 'sync;sync' 2>/dev/null || true
pkill -TERM qemu-system 2>/dev/null; sleep 3; pkill -9 qemu-system 2>/dev/null; sleep 2
NVKVM_M2CEFWD=1 nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3; pgrep qemu-system >/dev/null || { echo QEMU_DIED; cat /tmp/m0_launch.log; exit 1; }

echo "==> [3/5] wait for guest ssh"
up=0; for i in $(seq 1 48); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; echo "  up after $((i*5))s"; break; }; sleep 5; done
[ "$up" = 1 ] || { echo GUEST_NOBOOT; tail -8 /tmp/m0_launch.log; exit 1; }

echo "==> [4/5] push + run chshare"
scp -q -P $PORT -o StrictHostKeyChecking=no /workspace/nvkvm/tests/mode2/chshare.c ubuntu@localhost:/tmp/ 2>/dev/null
$SSHG "bash -s" < /workspace/nvkvm/scripts/mode2_diag/chshare_run_guest.sh 2>&1 | tail -40

echo "==> [5/5] host GPU util sample"
nvidia-smi --query-gpu=memory.used,utilization.gpu --format=csv,noheader | head -1
echo "==> DONE_CHSHARE"
