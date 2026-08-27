#!/usr/bin/env bash
# m584 — ON HOST. DIAG: does a no-exit KVM memslot work at all under this nested virt? The
# gsp_falcon page is currently FULL RW RAM (init_ram) — reads AND writes hit RAM, definitely a
# memslot. If mmio_exits craters -> memslots serve no-exit reads here (so the rom-device readonly
# path is the issue, pursue RW+ioeventfd-for-doorbell). If NOT -> KVM isn't memslotting this
# BAR-subregion at all under nested virt (deeper). (GSP writes lose side-effects -> run may fail;
# we only need the exit count.)
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=6 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
NVKVM_M2CEFWD=1 NVKVM_M2OPAQUE=1 NVKVM_M2ROMREGS=1 nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3; up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; break; }; sleep 5; done
echo "up=$up"
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  /workspace/nvkvm/scripts/mode2_diag/llm_run_guest.sh ubuntu@localhost:/tmp/ 2>/dev/null
QPID=$(pgrep -f qemu-system-x86_64 | head -1); KDIR=$(sudo bash -c "ls -d /sys/kernel/debug/kvm/${QPID}-* 2>/dev/null | head -1")
M0=$(sudo cat "$KDIR/mmio_exits" 2>/dev/null || echo 0)
$SSHG "LLM_NGEN=40 LLM_TIMEOUT=150 bash /tmp/llm_run_guest.sh" >/tmp/m584_llm.log 2>&1
M1=$(sudo cat "$KDIR/mmio_exits" 2>/dev/null || echo 0)
echo "mmio_exits during run (FULL-RAM gsp page): $((M1-M0))   [rom-device readonly was ~318k]"
echo "gen/rc: $(grep -aoE 'Generation: [0-9.]+ t/s|exit rc=[0-9]+' /tmp/m584_llm.log | tail -2 | tr '\n' ' ')"
