#!/usr/bin/env bash
# llm_run_guest.sh — ON THE GUEST. Mode-2 LLM inference proof: run a small Qwen2 GGUF
# fully on the host GPU (all layers offloaded) through the emulated GA106. Same
# module/firmware/device prep as cup4_run_guest.sh, then run the pre-staged CUDA
# llama.cpp (~/llm/llama-cli) against ~/llm/qwen.gguf. PASS = ggml_cuda_init succeeds,
# layers offload to GPU, and a coherent completion generates (vs Mode-1's ~20 tok/s).
# Un-forgeable end-to-end: many distinct ggml-cuda GR kernels + cuBLAS, large working
# set, sustained multi-token submission — far past cup4's single matmul kernel.
set -u
NVMODS=/home/ubuntu/nvmods
GUESTLIB=/usr/local/nvidia-guest/lib
LLM=$HOME/llm
MODEL=${LLM_MODEL:-$LLM/qwen.gguf}
NGEN=${LLM_NGEN:-32}
TIMEOUT=${LLM_TIMEOUT:-180}
PROMPT=${LLM_PROMPT:-Explain in two sentences why GPU virtualization is useful for cloud computing.}

sudo systemctl isolate multi-user.target 2>/dev/null || true
sleep 2
sudo rmmod nvidia_uvm nvidia nvkvm_guest 2>/dev/null || true
sudo modprobe ecdh_generic ecc 2>/dev/null || true
sudo dmesg -C || true
if [ ! -f /lib/firmware/nvidia/580.159.04/gsp_ga10x.bin ]; then
  sudo mkdir -p /lib/firmware/nvidia/580.159.04 /mnt/nvfw
  sudo mount -t 9p -o trans=virtio,version=9p2000.L,ro nvfw /mnt/nvfw 2>/dev/null || true
  sudo cp -n /mnt/nvfw/*.bin /lib/firmware/nvidia/580.159.04/ 2>/dev/null || true
  sudo umount /mnt/nvfw 2>/dev/null || true
fi
sudo insmod "$NVMODS/nvidia.ko" NVreg_EnableGpuFirmware=1 NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1 | tail -1 || true
sudo insmod "$NVMODS/nvidia-uvm.ko" 2>&1 | tail -1 || true
UVM_MAJ=$(awk '$2=="nvidia-uvm"{print $1}' /proc/devices)
sudo mknod /dev/nvidia0 c 195 0 2>/dev/null || true
sudo mknod /dev/nvidiactl c 195 255 2>/dev/null || true
if [ -n "$UVM_MAJ" ]; then
  sudo rm -f /dev/nvidia-uvm /dev/nvidia-uvm-tools
  sudo mknod /dev/nvidia-uvm c "$UVM_MAJ" 0 2>/dev/null || true
  sudo mknod /dev/nvidia-uvm-tools c "$UVM_MAJ" 1 2>/dev/null || true
fi
sudo chmod 666 /dev/nvidia* /dev/nvidiactl 2>/dev/null || true
sudo ln -sf "$GUESTLIB/libcuda.so.580.159.04" /lib/x86_64-linux-gnu/libcuda.so.1
sudo ln -sf "$GUESTLIB/libcuda.so.580.159.04" /usr/lib/x86_64-linux-gnu/libcuda.so 2>/dev/null; sudo ldconfig 2>/dev/null

echo "=== model ==="; ls -lah "$MODEL"
echo "=== llm lib (llama.cpp CUDA stack) ==="; ls "$LLM/lib"
echo "=== llama-cli run (ngl 99, n=$NGEN, timeout ${TIMEOUT}s) ==="
# LD_LIBRARY_PATH=$LLM/lib supplies cublas/cudart/ggml + libcuda 580.159.04 (the Mode-1
# recipe). -ngl 99 = all transformer layers in VRAM; -st = single-turn completion.
LD_LIBRARY_PATH="$LLM/lib" timeout --signal=INT "$TIMEOUT" stdbuf -oL -eL \
  "$LLM/llama-cli" -m "$MODEL" -ngl 99 -c 2048 -n "$NGEN" -st -p "$PROMPT" 2>&1
RC=$?
echo "=== llm exit rc=$RC (124=timeout/hang) ==="
