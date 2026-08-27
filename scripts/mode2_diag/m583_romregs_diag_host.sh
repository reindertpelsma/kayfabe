#!/usr/bin/env bash
# m583 — ON HOST. Does 0x110094 STILL trap (reach nvkvm_bar0_read) with the rom-device on? Boot
# romregs+trace, run the LLM (loads the driver -> GSP polling), then grep the BAR0-RD trace for
# 0x110094. If >0 -> the rom-device read isn't intercepting (KVM not serving the romd subregion).
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=6 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
NVKVM_M2CEFWD=1 NVKVM_M2OPAQUE=1 NVKVM_M2ROMREGS=1 NVKVM_M2TRACE=1 nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3; up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; exit 1; }
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  /workspace/nvkvm/scripts/mode2_diag/llm_run_guest.sh ubuntu@localhost:/tmp/ 2>/dev/null
$SSHG "LLM_NGEN=40 LLM_TIMEOUT=180 bash /tmp/llm_run_guest.sh" > /tmp/m583_llm.log 2>&1
echo "gen: $(grep -aoE 'Generation: [0-9.]+ t/s' /tmp/m583_llm.log | tail -1)"
echo "0x110094 reads STILL in bar0_read trace: $(grep -ac 'BAR0 RD  off=0x110094' /tmp/m0_qemu.log)"
echo "0x110118 (DMATRFCMD) reads in trace: $(grep -ac 'BAR0 RD  off=0x110118' /tmp/m0_qemu.log)"
echo "0x110c00 (QUEUE_HEAD wr, side-effect, SHOULD still trap): $(grep -ac 'off=0x110c00' /tmp/m0_qemu.log)"
echo "total BAR0 RD trace lines: $(grep -ac 'BAR0 RD  off=' /tmp/m0_qemu.log)"
echo "=> if 0x110094 count is ~0 the rom-device read WORKS (reads bypass bar0_read); if high it does NOT"
