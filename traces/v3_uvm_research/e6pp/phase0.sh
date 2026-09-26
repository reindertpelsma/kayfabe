#!/usr/bin/env bash
# E6'' phase 0 — on a STOCK box (nvidia-uvm loaded), record the exact nvidia-uvm
# ioctl surface of (a) tinygrad's libcuda-free NV backend and (b) libcuda, so the
# kf-uvm.ko impersonation surface is measured, not guessed. Container box is fine.
set -uo pipefail
cd /root/e6pp
exec > >(tee phase0.out) 2>&1
echo "PHASE0_START $(date -Is)"
cat /proc/driver/nvidia/version | head -1
nvidia-smi --query-gpu=name,driver_version,pci.device_id --format=csv,noheader
gcc -O2 -Wall -shared -fPIC -o nvioctl_log.so nvioctl_log.c -ldl && echo LOGGER_OK

echo "=== (b) libcuda: cuInit + alloc + launch (val.cu) ==="
nvcc -arch=sm_86 -rdc=true -o val val.cu k.cu 2>&1 | tail -2
rm -f log_libcuda.txt
NVLOG=$PWD/log_libcuda.txt LD_PRELOAD=$PWD/nvioctl_log.so ./val; echo "val_rc=$?"
echo "libcuda events: $(wc -l < log_libcuda.txt)"

echo "=== (a) tinygrad NV backend ==="
apt-get install -y -qq python3-pip git >/dev/null 2>&1
[ -d tinygrad ] || git clone -q --depth 1 https://github.com/tinygrad/tinygrad.git
git -C tinygrad log -1 --format='tinygrad %h %cd'
python3 -m pip install -q -e ./tinygrad 2>&1 | tail -1
rm -f log_tinygrad.txt
NVLOG=$PWD/log_tinygrad.txt LD_PRELOAD=$PWD/nvioctl_log.so NV=1 python3 -c "
from tinygrad import Tensor, Device
a = Tensor.arange(1024).realize(); b = (a * 2 + 1).realize()
v = b.tolist(); print('tinygrad', Device.DEFAULT, 'ok' if v[:3]==[1,3,5] and v[-1]==2047 else 'BAD', v[:3], v[-1])
m = (Tensor.ones(256,256) @ Tensor.ones(256,256)).realize().tolist()
print('matmul', 'ok' if m[0][0]==256.0 and m[255][255]==256.0 else 'BAD', m[0][0])
"; echo "tinygrad_rc=$?"
echo "tinygrad events: $(wc -l < log_tinygrad.txt)"

for f in log_libcuda.txt log_tinygrad.txt; do
  echo "=== $f: distinct UVM ioctls (count) ==="
  grep -o "UVM [A-Z_?]*([0-9]*)" $f | sort | uniq -c | sort -rn
  echo "=== $f: UVM non-zero rmStatus ==="
  grep "UVM " $f | grep -v "rmStatus=0x0 " | head
  echo "=== $f: RM alloc classes ==="
  grep -o "ALLOC class=0x[0-9a-f]* " $f | sort | uniq -c
  echo "=== $f: MAP_DMA ==="
  grep MAP_DMA $f | head -5
done
echo "PHASE0_DONE $(date -Is)"
