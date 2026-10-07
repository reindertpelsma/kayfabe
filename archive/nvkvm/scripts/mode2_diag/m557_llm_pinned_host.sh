#!/usr/bin/env bash
# m557_llm_pinned_host.sh — ON HOST. The real LLM-on-Mode-2 proof, now that cudart
# init works (0x2080900x cluster fix). Fresh boot, PIN the adapter (single GSP init —
# sidesteps the reload/WPR2 bug), load modules ONCE, run llama.cpp on the GPU. PASS =
# ggml_cuda_init succeeds, layers offload, coherent tokens generate, host util>0.
# DEFAULT mode. (QEMU already built with the cudart fix; no rebuild needed.)
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
QLOG=/tmp/m0_qemu.log
NGEN=${LLM_NGEN:-48}
TIMEOUT=${LLM_TIMEOUT:-180}

echo "==> fresh restart VM (DEFAULT mode)"
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; tail -3 /tmp/m0_launch.log; exit 1; }
echo "==> wait guest"
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; echo "  up ${i}x5s"; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; exit 1; }

echo "==> stage runner"
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  /workspace/nvkvm/tests/mode2/rtp.c /workspace/nvkvm/tests/mode2/reload_probe.c \
  /workspace/nvkvm/scripts/mode2_diag/rtp_run_guest.sh ubuntu@localhost:/tmp/ 2>/dev/null

echo "==> setup (load modules ONCE + PIN adapter)"
$SSHG 'bash /tmp/rtp_run_guest.sh setup' 2>&1 | tail -4

QB=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
echo "==> host GPU util sampler (background)"
( for t in $(seq 1 $((TIMEOUT/2))); do echo "util_t$((t*2))s: $(nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader 2>/dev/null)"; sleep 2; done ) > /tmp/m557_util.log 2>&1 &

echo "==> run llama.cpp on PINNED adapter (timeout ${TIMEOUT}s)"
$SSHG "LLM_NGEN=$NGEN LLM_TIMEOUT=$TIMEOUT bash /tmp/rtp_run_guest.sh llm" > /tmp/m557_guest.log 2>&1
QE=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
sed -n "$((QB+1)),${QE}p" "$QLOG" > /tmp/m557_delta.txt

echo ""; echo "============ M557 LLM-on-Mode-2 EVIDENCE ============"
echo "--- llama key lines ---"
grep -iE "cuda|ggml|offload|n_gpu_layers|llama_|tokens per second|eval time|exit rc|error|assert|abort|failed" /tmp/m557_guest.log | head -40
echo "--- generated text (tail) ---"; tail -16 /tmp/m557_guest.log
echo "--- host GPU util peak (LLM should sustain util>0) ---"; grep -oE "[0-9]+ %" /tmp/m557_util.log | sort -rn | head -3
echo "--- new CTRL-UNFILLED gates during llm (candidates if it failed) ---"; grep -E "CTRL-UNFILLED" /tmp/m557_delta.txt | grep -oE "cmd=0x[0-9a-f]+" | sort | uniq -c | sort -rn | head
echo "--- host Xid (want none new) ---"; sudo dmesg | grep -iE "Xid" | tail -3
echo "--- health ---"; uptime | grep -oE "load average.*"; ps -eo stat | grep -c "^D" | sed 's/^/  D-state: /'
echo "============ END ============"
