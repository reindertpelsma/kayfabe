#!/usr/bin/env bash
# m580_logoff_vmexit_host.sh — ON HOST. (1) Verify the M5.63 DIAG-log gate (m2trace default OFF) kills
# the ~360k-line/run spew. (2) Measure gen t/s with logging OFF + opaque ON. (3) Test the VMEXIT
# hypothesis (Mode-1 hit parity on this SAME nested box, so the 25-vs-63 gap is our per-doorbell MMIO
# trap, not the env): count KVM mmio_exits during generation via /sys/kernel/debug/kvm. If
# mmio_exits/token ~= doorbells/token and each nested vmexit is costly, that's the real wall (opacity
# alone can't fix it — needs doorbell PASSTHROUGH, the Mode-1 model).
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=6 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
NGEN=${LLM_NGEN:-96}; TIMEOUT=${LLM_TIMEOUT:-240}
SRC=/workspace/nvkvm/src/qemu/nvkvm_gpu_emul.c

cp "$SRC" /opt/qemu-src/hw/misc/nvkvm_gpu_emul.c
( cd /opt/qemu-src/build && ninja install ) 2>&1 | tail -2 || { echo BUILD_FAIL; exit 1; }

pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
NVKVM_M2CEFWD=1 NVKVM_M2OPAQUE=1 nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; tail -4 /tmp/m0_launch.log; exit 1; }
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; exit 1; }
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  /workspace/nvkvm/scripts/mode2_diag/llm_run_guest.sh ubuntu@localhost:/tmp/ 2>/dev/null

QPID=$(pgrep -f qemu-system-x86_64 | head -1)
KDIR=$(sudo bash -c "ls -d /sys/kernel/debug/kvm/${QPID}-* 2>/dev/null | head -1")
rd() { sudo cat "$KDIR/$1" 2>/dev/null || echo 0; }
MMIO0=$(rd mmio_exits); IO0=$(rd io_exits); HALT0=$(rd halt_exits); INS0=$(rd insn_emulation 2>/dev/null)
LOG0=$(wc -l < /tmp/m0_qemu.log 2>/dev/null || echo 0)

$SSHG "LLM_NGEN=$NGEN LLM_TIMEOUT=$TIMEOUT bash /tmp/llm_run_guest.sh" > /tmp/m580_llm.log 2>&1

MMIO1=$(rd mmio_exits); IO1=$(rd io_exits); HALT1=$(rd halt_exits)
LOG1=$(wc -l < /tmp/m0_qemu.log 2>/dev/null || echo 0)
GEN=$(grep -aoE 'Generation: [0-9.]+ t/s' /tmp/m580_llm.log | tail -1)
NG=$(grep -aoE 'Generation: [0-9.]+' /tmp/m580_llm.log | tail -1 | grep -oE '[0-9.]+')

echo "############ M580 logging-gate + vmexit (opaque on, trace OFF) ############"
echo "KVM dir: $KDIR"
echo "LLM: $GEN  rc=$(grep -aoE 'exit rc=[0-9]+' /tmp/m580_llm.log | tail -1)"
echo "QEMU log lines during run: $((LOG1-LOG0))   (was ~360000 with trace on -> gate working if tiny)"
echo "mmio_exits during run: $((MMIO1-MMIO0))   io_exits: $((IO1-IO0))   halt_exits: $((HALT1-HALT0))"
echo "  Xid: $(grep -ac 'Xid' /tmp/m0_qemu.log)  OPAQUE skip-walk doorbells: $(grep -ac 'OPAQUE skip-walk' /tmp/m0_qemu.log 2>/dev/null || echo 'n/a(trace off)')"
echo "--- coherence ---"; grep -aiE 'GPU|cloud|the ' /tmp/m580_llm.log | grep -avE 'Generation|rc=|===|t/s|ggml|load' | tail -2
echo "--- health ---"; uptime | grep -oE "load average.*"; ps -eo stat | grep -c '^D' | sed 's/^/D-state: /'
echo "############ END m580 ############"
