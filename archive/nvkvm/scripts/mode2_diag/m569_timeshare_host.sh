#!/usr/bin/env bash
# m569_timeshare_host.sh — ON HOST. Perf step 1: MEASURE the Mode-2 time-share before
# building CE-forward. Boots the map-on-touch build with NVKVM_M2CEFWD=1, runs the small
# Qwen2 LLM to completion, then surfaces the NVKVM-TIMESHARE log lines (wall-clock ns +
# % in each hot path: emulated-CE byte copy, guest-CPU PRAMIN-window read/write traps,
# doorbell re-sweep, doorbell forward, chan_execute). The LAST line is the steady-state
# reading. Decides whether CE-forward alone gets us toward 60 tok/s or if the window
# data path / submission rate dominate. PASS = run completes + a dominant bucket is clear.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
QLOG=/tmp/m0_qemu.log
NGEN=${LLM_NGEN:-32}
TIMEOUT=${LLM_TIMEOUT:-240}

echo "==> install QEMU (already built by ninja)"
( cd /opt/qemu-src/build && ninja install ) 2>&1 | tail -1 || { echo INSTALL_FAIL; exit 1; }

echo "==> fresh restart VM (NVKVM_M2CEFWD=1 — map-on-touch + time-share instrumented)"
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
NVKVM_M2CEFWD=1 nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; tail -3 /tmp/m0_launch.log; exit 1; }

echo "==> wait guest"
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; echo "  up ${i}x5s"; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; exit 1; }
echo "==> guest kernel (must be 6.8.0-117):"; $SSHG uname -r 2>/dev/null

echo "==> stage runner"
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  /workspace/nvkvm/scripts/mode2_diag/llm_run_guest.sh ubuntu@localhost:/tmp/ 2>/dev/null
QB=$(wc -l < "$QLOG" 2>/dev/null || echo 0)

echo "==> run llm to completion (foreground, timeout $((TIMEOUT+20))s)"
$SSHG "LLM_NGEN=$NGEN LLM_TIMEOUT=$TIMEOUT bash /tmp/llm_run_guest.sh" > /tmp/m569_guest.log 2>&1
echo "--- llm exit rc + token rate ---"
grep -iE "exit rc|tokens per second|eval time" /tmp/m569_guest.log | tail -6

QE=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
sed -n "$((QB+1)),${QE}p" "$QLOG" > /tmp/m569_delta.txt
D=/tmp/m569_delta.txt

echo ""; echo "============ M569 TIME-SHARE EVIDENCE ============"
echo "--- *** NVKVM-TIMESHARE samples (LAST = steady-state) ---"
grep "NVKVM-TIMESHARE" $D | tail -6
echo "--- final DPLANE summary (volumes) ---"; grep "NVKVM-DPLANE SUMMARY" $D | tail -1
echo "--- doorbell count / CE launchdma count in delta ---"
grep -c "exec_doorbell GR gp_get" $D | sed 's/^/  exec_doorbell forwards: /'
grep -c "NVKVM-DPLANE CE-LAUNCHDMA" $D | sed 's/^/  CE-LAUNCHDMA (capped log): /'
grep -c "M5.10 doorbell re-sweep" $D | sed 's/^/  re-sweeps: /'
echo "--- map-on-touch signals ---"
grep -c "gpga_obj\[gpu_only\]" $D | sed 's/^/  gpu_only objs: /'
grep -c "map-on-touch PROMOTED" $D | sed 's/^/  promoted: /'
grep -c "gpga_obj FAILED" $D | sed 's/^/  gpga FAILED: /'
echo "--- host Xid (PER-RUN; cross-check ts) ---"; sudo dmesg | grep -iE "Xid" | tail -3
echo "--- GPU health ---"; uptime | grep -oE "load average.*"; ps -eo stat | grep -c '^D' | sed 's/^/  D-state: /'
echo "============ END ============"
