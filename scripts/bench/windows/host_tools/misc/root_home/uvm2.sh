#!/bin/bash
echo "-- /proc/modules (nvidia/nvkvm) --"
grep -E '^nvidia|^nvkvm' /proc/modules 2>/dev/null | awk '{print "   "$1" refs="$3}' || echo "   (none listed)"
echo "-- nvidia-smi -L --"
timeout 20 nvidia-smi -L 2>&1 | head -3
echo "-- uvm node openable (timeout-guarded) --"
timeout 5 dd if=/dev/nvidia-uvm of=/dev/null bs=1 count=0 2>&1 | tail -1
echo "   exit=$?"
echo "-- dmesg --"
dmesg 2>/dev/null | grep -iE 'nvkvm|uvm|NVRM' | tail -6 || echo "   (dmesg unavailable in container)"
