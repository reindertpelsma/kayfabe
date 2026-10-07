#!/usr/bin/env bash
# m582_romregs_host.sh — ON HOST. Validate the M5.64 GSP-falcon rom-device overlay (m2romregs): the
# 0x110094/IRQSTAT/DMATRFCMD poll-reads should now hit RAM (no vmexit). GATE = correctness (must NOT
# break GSP boot or compute): LLM coherent + rc=0 + Xid=0. WIN = mmio_exits/run craters vs m580's 320k.
# A/B: RUN1 romregs ON, RUN2 romregs OFF (control), same cefwd+opaque, fresh boot each.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=6 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
NGEN=${LLM_NGEN:-96}; TIMEOUT=${LLM_TIMEOUT:-240}
SRC=/workspace/nvkvm/src/qemu/nvkvm_gpu_emul.c
cp "$SRC" /opt/qemu-src/hw/misc/nvkvm_gpu_emul.c
( cd /opt/qemu-src/build && ninja install ) 2>&1 | tail -1 || { echo BUILD_FAIL; exit 1; }

run() {  # $1=label  $2=extra-env
  pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
  env NVKVM_M2CEFWD=1 NVKVM_M2OPAQUE=1 $2 nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
  sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo "$1 QEMU_DIED"; return; }
  local up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; break; }; sleep 5; done
  [ "$up" = 1 ] || { echo "$1 NOBOOT"; return; }
  scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
    /workspace/nvkvm/scripts/mode2_diag/llm_run_guest.sh ubuntu@localhost:/tmp/ 2>/dev/null
  local QPID=$(pgrep -f qemu-system-x86_64 | head -1)
  local KDIR=$(sudo bash -c "ls -d /sys/kernel/debug/kvm/${QPID}-* 2>/dev/null | head -1")
  local M0=$(sudo cat "$KDIR/mmio_exits" 2>/dev/null || echo 0)
  $SSHG "LLM_NGEN=$NGEN LLM_TIMEOUT=$TIMEOUT bash /tmp/llm_run_guest.sh" > /tmp/m582_$1.log 2>&1
  local M1=$(sudo cat "$KDIR/mmio_exits" 2>/dev/null || echo 0)
  echo "[$1] $(grep -aoE 'Generation: [0-9.]+ t/s' /tmp/m582_$1.log | tail -1)  rc=$(grep -aoE 'exit rc=[0-9]+' /tmp/m582_$1.log | tail -1)"
  echo "    mmio_exits during run: $((M1-M0))   Xid: $(grep -ac 'Xid' /tmp/m0_qemu.log)"
  echo "    coherence: $(grep -aiE 'GPU|cloud' /tmp/m582_$1.log | grep -avE 'Generation|rc=|===|ggml|load|t/s' | tail -1)"
}

echo "############ M582 GSP-falcon rom-device A/B ############"
echo "RUN1 romregs=ON :"; run romON  "NVKVM_M2ROMREGS=1"
echo "RUN2 romregs=OFF:"; run romOFF ""
echo "RUN3 romregs=ON :"; run romON2 "NVKVM_M2ROMREGS=1"
echo "--- health ---"; uptime | grep -oE "load average.*"; ps -eo stat | grep -c '^D' | sed 's/^/D-state: /'
echo "############ END m582 ############"
