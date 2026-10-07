#!/usr/bin/env bash
# m568_llm_maptouch_host.sh — ON HOST. North-star step 3 WITH map-on-touch: run a small Qwen2
# GGUF fully on the host GPU through the emulated GA106, but boot with NVKVM_M2CEFWD=1 so the
# large weight/KV buffers get gpu_only-by-default backing (off host BAR1) + promote-on-touch.
# This is the real test of map-on-touch at LLM scale (m553 ran DEFAULT mode). PASS = CUDA
# inits, all layers offload, coherent tokens generate, no hang/Xid, and the map-on-touch
# signals (gpu_only objs, PROMOTED, GIVE-UP) show the off-BAR1 path carried the weights.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
QLOG=/tmp/m0_qemu.log
NGEN=${LLM_NGEN:-32}
TIMEOUT=${LLM_TIMEOUT:-240}

echo "==> build QEMU"
( cd /opt/qemu-src/build && ninja qemu-system-x86_64 && ninja install ) 2>&1 | tail -2 || { echo BUILD_FAIL; exit 1; }

echo "==> fresh restart VM (NVKVM_M2CEFWD=1 — map-on-touch)"
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

echo "==> host GPU util sampler (background, ~${TIMEOUT}s)"
( for t in $(seq 1 $((TIMEOUT/2))); do echo "util_t$((t*2))s: $(nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader 2>/dev/null)"; sleep 2; done ) > /tmp/m568_util.log 2>&1 &

echo "==> run llm to completion (foreground, timeout $((TIMEOUT+20))s)"
$SSHG "LLM_NGEN=$NGEN LLM_TIMEOUT=$TIMEOUT bash /tmp/llm_run_guest.sh" > /tmp/m568_guest.log 2>&1
echo "--- llm stdout (key lines) ---"
grep -iE "cuda|ggml|offload|llama_|n_gpu_layers|tokens per second|eval time|exit rc|error|assert|abort" /tmp/m568_guest.log | head -50
echo "--- generated text (tail) ---"; tail -24 /tmp/m568_guest.log

QE=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
sed -n "$((QB+1)),${QE}p" "$QLOG" > /tmp/m568_delta.txt
D=/tmp/m568_delta.txt
echo ""; echo "============ M568 LLM + MAP-ON-TOUCH EVIDENCE ============"
echo "--- llm exit rc (124=hang) + token rate ---"; grep -iE "exit rc|tokens per second|eval time" /tmp/m568_guest.log | tail -5
echo "--- *** gpu_only backings (off-BAR1 weight/KV leaves) ---"
grep -c "gpga_obj\[gpu_only\]" $D | sed 's/^/  gpu_only objs: /'
echo "--- *** PROMOTED (lazy CPU map on touch) ---"; grep -c "map-on-touch PROMOTED" $D | sed 's/^/  promoted: /'
echo "--- GIVE-UP (BAR1 full at touch -> fb_pages) ---"; grep -c "map-on-touch GIVE-UP" $D | sed 's/^/  give-up: /'
grep "map-on-touch GIVE-UP" $D | tail -4
echo "--- gpga_obj cpu-mapped vs gpu_only + FAILURES ---"
grep -c "M7 R2 gpga_obj:" $D | sed 's/^/  gpga (cpu-mapped): /'
grep -c "gpga_obj\[gpu_only\]" $D | sed 's/^/  gpga (gpu_only): /'
grep -c "gpga_obj FAILED" $D | sed 's/^/  gpga FAILED: /'
grep "gpga_obj FAILED" $D | tail -3
echo "--- faults / NO_MEMORY / st=0x57 in QEMU delta ---"; grep -cE "FAULT|NO_MEMORY|st=0x57" $D | sed 's/^/  count: /'
echo "--- util peak / sustained ---"; grep -oE "[0-9]+ %" /tmp/m568_util.log | sort -rn | head -3; tail -2 /tmp/m568_util.log
echo "--- host Xid (PER-RUN; dmesg not cleared across boots — cross-check ts) ---"; sudo dmesg | grep -iE "Xid" | tail -4
echo "--- GPU health ---"; uptime | grep -oE "load average.*"; ps -eo stat | grep -c '^D' | sed 's/^/  D-state: /'
echo "============ END ============"
