#!/bin/bash
echo "-- modules in the kata guest --"
lsmod 2>/dev/null | grep -E '^nvidia|^nvkvm' | awk '{print "   "$1" refs="$3}'
echo "-- can we open the uvm node? --"
for d in /dev/nvidia-uvm /dev/nvidia0 /dev/nvidiactl; do
  if exec 9<>"$d" 2>/dev/null; then echo "   $d open OK"; exec 9>&-; else echo "   $d open FAILED: $?"; fi
done
echo "-- nvidia-smi device listing --"
nvidia-smi -L 2>&1 | head -3
echo "-- kernel messages mentioning nvkvm/uvm/nvrm --"
dmesg 2>/dev/null | grep -iE 'nvkvm|uvm|nvrm' | tail -8
