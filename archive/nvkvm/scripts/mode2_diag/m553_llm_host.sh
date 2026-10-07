#!/usr/bin/env bash
# m553_llm_host.sh — ON HOST. Mode-2 LLM inference proof (north-star step 3): run a
# small Qwen2 GGUF fully on the host GPU through the emulated GA106 + faked GSP, the
# way it ran on Mode-1 (#27). DEFAULT mode. Fresh boot, run llm_run_guest.sh to
# completion. PASS = CUDA inits, layers offload, coherent tokens generate. Captures
# util (LLM should sustain util>0, unlike the microsecond matmul) + any new faults.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
QLOG=/tmp/m0_qemu.log
NGEN=${LLM_NGEN:-32}
TIMEOUT=${LLM_TIMEOUT:-180}

echo "==> build QEMU"
( cd /opt/qemu-src/build && ninja qemu-system-x86_64 && ninja install ) 2>&1 | tail -2 || { echo BUILD_FAIL; exit 1; }

echo "==> fresh restart VM (DEFAULT mode)"
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; tail -3 /tmp/m0_launch.log; exit 1; }

echo "==> wait guest"
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; echo "  up ${i}x5s"; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; exit 1; }
echo "==> guest kernel (must be 6.8.0-117):"; $SSHG uname -r 2>/dev/null

echo "==> stage runner"
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  /workspace/nvkvm/scripts/mode2_diag/llm_run_guest.sh ubuntu@localhost:/tmp/ 2>/dev/null
QB=$(wc -l < "$QLOG" 2>/dev/null || echo 0)

echo "==> host GPU util sampler (background, ~${TIMEOUT}s)"
( for t in $(seq 1 $((TIMEOUT/2))); do echo "util_t$((t*2))s: $(nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader 2>/dev/null)"; sleep 2; done ) > /tmp/m553_util.log 2>&1 &

echo "==> run llm to completion (foreground, timeout $((TIMEOUT+20))s)"
$SSHG "LLM_NGEN=$NGEN LLM_TIMEOUT=$TIMEOUT bash /tmp/llm_run_guest.sh" > /tmp/m553_guest.log 2>&1
echo "--- llm stdout (key lines) ---"
grep -iE "cuda|ggml|offload|llama_|n_gpu_layers|tokens per second|eval time|exit rc|error|assert|abort" /tmp/m553_guest.log | head -50
echo "--- generated text (tail) ---"; tail -20 /tmp/m553_guest.log

QE=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
sed -n "$((QB+1)),${QE}p" "$QLOG" > /tmp/m553_delta.txt
D=/tmp/m553_delta.txt
echo ""; echo "============ M553 LLM EVIDENCE ============"
echo "--- util peak / sustained ---"; grep -oE "[0-9]+ %" /tmp/m553_util.log | sort -rn | head -5; tail -3 /tmp/m553_util.log
echo "--- host Xid (want NONE new) ---"; sudo dmesg | grep -iE "Xid" | tail -5
echo "--- faults / NO_MEMORY / st=0x57 in QEMU delta ---"; grep -cE "FAULT|NO_MEMORY|st=0x57" $D | sed 's/^/  count: /'
echo "--- M5.51/M5.52 back failures (want 0) ---"; grep -cE "gpga_obj FAILED|M5.51.*FAIL" $D | sed 's/^/  count: /'
echo "--- GPU health ---"; uptime | grep -oE "load average.*"; ps -eo stat | grep -c '^D' | sed 's/^/  D-state: /'
echo "============ END ============"
