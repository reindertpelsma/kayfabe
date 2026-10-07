#!/usr/bin/env bash
# m585 — ON HOST. Establish the HOST-NATIVE ceiling (≈ Mode-1 reference, ~0.97x) for the SAME small
# qwen.gguf Mode-2 runs, so we can express Mode-2 t/s as a fraction of the ceiling and size the (b)
# lever. Boot the Mode-2 guest only to extract ~/llm (model + CUDA llama-cli), then kill the guest
# and run llama-cli NATIVELY on the host RTX 3060 (no VM, no forwarding). Same flags as Mode-2.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=6 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
NGEN=${LLM_NGEN:-96}
PROMPT="Explain in two sentences why GPU virtualization is useful for cloud computing."

if [ ! -f /tmp/hostllm/llama-cli ] || [ ! -f /tmp/hostllm/qwen.gguf ]; then
  echo "==> boot guest to extract ~/llm"
  pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
  NVKVM_M2CEFWD=1 nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
  sleep 3; up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; break; }; sleep 5; done
  [ "$up" = 1 ] || { echo NOBOOT; exit 1; }
  mkdir -p /tmp/hostllm
  echo "==> guest ~/llm contents:"; $SSHG 'ls -lah ~/llm 2>/dev/null' 2>/dev/null
  scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
    -r ubuntu@localhost:llm/'*' /tmp/hostllm/ 2>/dev/null
fi
echo "==> free the GPU (kill guest)"; pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
ls -lah /tmp/hostllm/ | head
chmod +x /tmp/hostllm/llama-cli 2>/dev/null
echo "==> HOST-NATIVE run on RTX 3060 (ngl 99, n=$NGEN)"
nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader
LD_LIBRARY_PATH=/usr/lib/x86_64-linux-gnu:/tmp/hostllm timeout 240 \
  /tmp/hostllm/llama-cli -m /tmp/hostllm/qwen.gguf -ngl 99 -c 2048 -n "$NGEN" -st -p "$PROMPT" \
  > /tmp/m585_host.log 2>&1
echo "rc=$?"
echo "=== host-native timing (eval/gen tokens per second) ==="
grep -iE 'tokens per second|eval time|Generation|t/s|llama_perf' /tmp/m585_host.log | tail -8
echo "=== sample output (coherence) ==="; grep -iE 'GPU|cloud|the ' /tmp/m585_host.log | grep -avE 'time|perf|t/s|=' | tail -2
echo "--- health ---"; nvidia-smi --query-gpu=utilization.gpu --format=csv,noheader
echo "############ END m585 ############"
