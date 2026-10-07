#!/usr/bin/env bash
# m573_genprofile_host.sh — ON HOST. Profile the REAL GPU-accelerated generation phase to find
# what the single busy guest vCPU actually burns cycles on (m571: gen is ~1 vCPU busy, 75% idle,
# host GPU util~0 -> serialized per-op cost, NOT traps/cache). MUST use llm_run_guest.sh's full
# Mode-2 setup (insmod nvidia/uvm, device nodes, firmware, libcuda symlink) or CUDA fails init and
# llama silently falls back to CPU matmul (the m572 artifact). Fresh boot (stale GSP otherwise).
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=6 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
echo "==> install + fresh boot (NVKVM_M2CEFWD=1, page-batch build)"
( cd /opt/qemu-src/build && ninja install ) 2>&1 | tail -1
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
NVKVM_M2CEFWD=1 nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; exit 1; }
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; exit 1; }
echo "  guest up"

# Guest profiler: real setup (from llm_run_guest.sh) -> background llama -> perf during gen.
cat > /tmp/m573_guest.sh <<'GEOF'
#!/usr/bin/env bash
set -u
NVMODS=/home/ubuntu/nvmods; GUESTLIB=/usr/local/nvidia-guest/lib; LLM=$HOME/llm
echo -1 | sudo tee /proc/sys/kernel/perf_event_paranoid >/dev/null 2>&1
sudo systemctl isolate multi-user.target 2>/dev/null || true; sleep 2
sudo rmmod nvidia_uvm nvidia nvkvm_guest 2>/dev/null || true
sudo modprobe ecdh_generic ecc 2>/dev/null || true; sudo dmesg -C || true
sudo insmod "$NVMODS/nvidia.ko" NVreg_EnableGpuFirmware=1 NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1 | tail -1 || true
sudo insmod "$NVMODS/nvidia-uvm.ko" 2>&1 | tail -1 || true
UVM_MAJ=$(awk '$2=="nvidia-uvm"{print $1}' /proc/devices)
sudo mknod /dev/nvidia0 c 195 0 2>/dev/null||true; sudo mknod /dev/nvidiactl c 195 255 2>/dev/null||true
if [ -n "$UVM_MAJ" ]; then sudo rm -f /dev/nvidia-uvm /dev/nvidia-uvm-tools
  sudo mknod /dev/nvidia-uvm c "$UVM_MAJ" 0 2>/dev/null||true; sudo mknod /dev/nvidia-uvm-tools c "$UVM_MAJ" 1 2>/dev/null||true; fi
sudo chmod 666 /dev/nvidia* /dev/nvidiactl 2>/dev/null || true
sudo ln -sf "$GUESTLIB/libcuda.so.580.159.04" /lib/x86_64-linux-gnu/libcuda.so.1
# CONTROL-PATH latency probe (user lead: nvidia-smi ~5s). Time a couple of nvidia-smi calls on the
# freshly-loaded driver — this exercises the RM-control/GSP-RPC forward path that cudaMemGetInfo
# (hot during llama load) also uses. A multi-second result localizes the load bottleneck.
if command -v nvidia-smi >/dev/null 2>&1; then
  for n in 1 2; do
    t0=$(date +%s.%N); nvidia-smi -L >/tmp/smi$n.out 2>&1; t1=$(date +%s.%N)
    echo "nvidia-smi #$n: $(echo "$t1-$t0"|bc)s -> $(head -1 /tmp/smi$n.out)"
  done
else echo "no nvidia-smi"; fi
PROMPT="Explain in two sentences why GPU virtualization is useful for cloud computing."
( LD_LIBRARY_PATH="$LLM/lib" "$LLM/llama-cli" -m "$LLM/qwen.gguf" -ngl 99 -c 2048 -n 1200 -st -p "$PROMPT" >/tmp/m573_llama.log 2>&1 ) &
# wait for GENERATION on the GPU path: ggml_cuda_init must succeed + coherent text streaming
prev=0; gen=0; cudaok=0
for i in $(seq 1 70); do sleep 1
  grep -q "ggml_cuda_init: found" /tmp/m573_llama.log 2>/dev/null && cudaok=1
  sz=$(wc -c < /tmp/m573_llama.log 2>/dev/null||echo 0)
  letters=$(tail -c 120 /tmp/m573_llama.log 2>/dev/null | tr -dc 'a-zA-Z' | wc -c)
  if [ "$sz" -gt 1100 ] && [ "$letters" -gt 25 ] && [ "$sz" -gt "$prev" ]; then
    gen=$((gen+1)); [ "$gen" -ge 2 ] && { echo "GEN at ${i}s (cudaok=$cudaok sz=$sz)"; break; }
  else gen=0; fi; prev=$sz
done
echo "=== CUDA init status ==="; grep -iE "ggml_cuda_init|no usable GPU|offloaded|CUDA0" /tmp/m573_llama.log | head -6
LPID=$(pgrep -f llama-cli | head -1)
echo "hot-thread state=$(top -H -b -n1 -p $LPID 2>/dev/null | awk 'NR>7{print $1,$9}'|sort -k2 -rn|head -1)"
echo "=== perf record dwarf 6s during GPU-path generation ==="
sudo perf record -F 1500 -g --call-graph dwarf -p "$LPID" -o /tmp/m573.data -- sleep 6 >/dev/null 2>&1
echo "--- hottest SELF symbols ---"
sudo perf report -i /tmp/m573.data --stdio --no-children 2>/dev/null | grep -vE '^#|^$' | head -22
echo "--- top call chains (children) ---"
sudo perf report -i /tmp/m573.data --stdio 2>/dev/null | grep -vE '^#|^$' | head -16
echo "tail: ...$(tail -c 100 /tmp/m573_llama.log | tr '\n' ' ')"
sudo pkill -9 -f llama-cli 2>/dev/null; echo "cleaned=$(pgrep -c llama-cli)"
GEOF
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null /tmp/m573_guest.sh ubuntu@localhost:/tmp/ 2>/dev/null
echo "============ M573 REAL GEN-PATH PROFILE ============"
$SSHG "bash /tmp/m573_guest.sh" 2>&1
echo "--- host GPU util during (sampled) ---"; nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader
echo "--- host TIMESHARE ---"; grep "NVKVM-TIMESHARE" /tmp/m0_qemu.log | tail -1
echo "============ END ============"
