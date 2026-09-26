#!/usr/bin/env bash
set -uo pipefail
export PATH=/usr/local/cuda-12.6/bin:$PATH
cd /root/e6p
echo "== nvcc =="; nvcc --version | tail -2
echo "== compile cubin (sm_86, GA10x) =="
nvcc -arch=sm_86 -cubin -o k.cubin k.cu 2>&1 | tail -5 && echo "CUBIN_OK $(ls -l k.cubin|awk '{print $5}')B"
echo "== compile ptx =="; nvcc -arch=sm_86 -ptx -o k.ptx k.cu 2>&1|tail -3
echo "== cuobjdump -elf (resource usage, param layout) =="
cuobjdump -elf k.cubin 2>&1 | grep -iE "REG|BAR|SHARED|LOCAL|CONSTANT|PARAM|EIATTR|\.nv\.info|\.text\.k|FRAME|MIN_STACK|SASS" | head -40
echo "== cuobjdump -sass (entry, param loads, addr of program) =="
cuobjdump -sass k.cubin 2>&1 | sed -n '1,60p'
echo "== nvdisasm headers (section addrs) =="
cuobjdump -elf k.cubin 2>&1 | grep -iE "index|offset|size|align|\.text|\.nv\.constant|\.nv\.info|SHF|type" | head -40
echo "KERNEL_BUILD_DONE"
