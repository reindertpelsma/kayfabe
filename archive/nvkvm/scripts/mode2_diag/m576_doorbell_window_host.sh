#!/usr/bin/env bash
# m576_doorbell_window_host.sh — ON HOST. The DECISIVE submit-vs-spin split for the 22->60 tok/s
# lever. Adds a per-WINDOW timeshare dump (NVKVM-TWIN) keyed to doorbell count so steady-state
# GENERATION is sampled (the cumulative dump was load-dominated). Each window reports:
#   INSIDE_qemu%  = wall fraction the vCPU is blocked in exec_doorbell + chan_execute + os-event
#                   delivery (the SUBMIT-side cost we CAN cut in QEMU).
#   OUTSIDE%      = wall - inside = guest launch overhead + libcuda's cuStreamSynchronize SPIN
#                   waiting for the completion sema (H2 = completion-visibility latency).
# READ: if OUTSIDE dominates with host GPU idle -> H2 confirmed, the lever is completion delivery
# (not QEMU submit). If INSIDE dominates (db us/doorbell ~ ms) -> the lever is exec_doorbell cost.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=6 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
NGEN=${LLM_NGEN:-96}; TIMEOUT=${LLM_TIMEOUT:-240}
SRC=/workspace/nvkvm/src/qemu/nvkvm_gpu_emul.c

echo "==> sync device + build"
cp "$SRC" /opt/qemu-src/hw/misc/nvkvm_gpu_emul.c
( cd /opt/qemu-src/build && ninja install ) 2>&1 | tail -3 || { echo BUILD_FAIL; exit 1; }

echo "==> fresh boot (NVKVM_M2CEFWD=1)"
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
NVKVM_M2CEFWD=1 nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; tail -5 /tmp/m0_launch.log; exit 1; }
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; exit 1; }
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  /workspace/nvkvm/scripts/mode2_diag/llm_run_guest.sh ubuntu@localhost:/tmp/ 2>/dev/null

echo "==> run LLM (NGEN=$NGEN)"
$SSHG "LLM_NGEN=$NGEN LLM_TIMEOUT=$TIMEOUT bash /tmp/llm_run_guest.sh" > /tmp/m576_llm.log 2>&1
echo "--- gen result ---"; grep -iE "Generation:|exit rc=|Prompt:" /tmp/m576_llm.log | tail -3

QLOG=$(ls -t /tmp/m0_qemu.log 2>/dev/null | head -1)
echo "==> NVKVM-TWIN windows (qlog=$QLOG):"
grep -a "NVKVM-TWIN" "$QLOG" 2>/dev/null | tail -25
echo "==> last cumulative TIMESHARE:"
grep -a "NVKVM-TIMESHARE" "$QLOG" 2>/dev/null | tail -2
echo "--- health ---"; uptime | grep -oE "load average.*"; ps -eo stat | grep -c '^D' | sed 's/^/D-state: /'
echo "############ END m576 ############"
