#!/usr/bin/env bash
# m581_genphase_traps_host.sh — ON HOST. Which BAR0 regs trap during GENERATION vs LOAD? 0x110094
# (NV_PGSP_FALCON_DEBUGINFO) dominates the whole-run histogram, but if it's GSP-RPC-response polling
# it's a LOAD-phase cost, not gen. Boot trace ON (m2trace), longer gen (NGEN=150), then histogram BAR
# RD/WR offsets in the FIRST half (load/boot) vs LAST third (steady gen) of the trace. The gen-half
# top offsets are the real per-token vmexit drivers to make non-trapping (RO memslot / passthrough).
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=6 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
NGEN=${LLM_NGEN:-150}; TIMEOUT=${LLM_TIMEOUT:-300}

( cd /opt/qemu-src/build && ninja install ) 2>&1 | tail -1
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
NVKVM_M2CEFWD=1 NVKVM_M2OPAQUE=1 NVKVM_M2TRACE=1 nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; tail -4 /tmp/m0_launch.log; exit 1; }
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; exit 1; }
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  /workspace/nvkvm/scripts/mode2_diag/llm_run_guest.sh ubuntu@localhost:/tmp/ 2>/dev/null

# Mark the qemu-log position right before generation by reading the line count after model load.
# Heuristic split: BAR-trace lines are emitted continuously; load is the head, gen the tail.
$SSHG "LLM_NGEN=$NGEN LLM_TIMEOUT=$TIMEOUT bash /tmp/llm_run_guest.sh" > /tmp/m581_llm.log 2>&1
echo "gen: $(grep -aoE 'Generation: [0-9.]+ t/s' /tmp/m581_llm.log | tail -1)"

QL=/tmp/m0_qemu.log
TOT=$(grep -ac 'BAR0 RD  off=' "$QL")
echo "total BAR0 RD trace lines: $TOT"
echo "=== HEAD (first 40% = boot/load) top offsets ==="
grep -a 'BAR0 RD  off=' "$QL" | head -n $((TOT*4/10)) | grep -aoE 'off=0x[0-9a-f]+' | sort | uniq -c | sort -rn | head -6
echo "=== TAIL (last 30% = steady gen) top offsets ==="
grep -a 'BAR0 RD  off=' "$QL" | tail -n $((TOT*3/10)) | grep -aoE 'off=0x[0-9a-f]+' | sort | uniq -c | sort -rn | head -6
echo "=== TAIL BAR0 WR (gen writes = side-effect doorbells) ==="
grep -a 'BAR0 WR  off=' "$QL" | tail -n $((TOT*3/10)) | grep -aoE 'off=0x[0-9a-f]+' | sort | uniq -c | sort -rn | head -5
echo "--- health ---"; uptime | grep -oE "load average.*"; ps -eo stat | grep -c '^D' | sed 's/^/D-state: /'
echo "############ END m581 ############"
