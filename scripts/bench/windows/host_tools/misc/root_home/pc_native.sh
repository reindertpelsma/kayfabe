#!/bin/bash
set -x
for p in $(ps -eo pid,args | grep "[q]emu-system-x86_64" | awk '{print $1}'); do kill -9 $p 2>/dev/null; done
sleep 2
rm -f /tmp/pc_native.log
cd /srv/nvkvm-pv
chmod 755 /srv/nvkvm-pv 2>/dev/null
env \
  XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-0 \
  VM_IMG=/opt/nvkvm-guest/mint-22.3.qcow2 VM_SEED= \
  VM_DISPLAY="sdl,gl=on" VM_RELATIVE_MOUSE=1 VM_SERIAL=none VM_MEM=16G VM_SMP=8 \
  NVKVM_PRESENT_TIMING=1 \
  setsid bash scripts/run_test_vm.sh > /tmp/pc_native.log 2>&1 < /dev/null &
sleep 25
ps -eo user,pid,args | grep "[q]emu-system-x86_64" | head -1 | cut -c1-140
echo "--- present lines ---"
grep -E "display mode|host EGL|egl_init failed|eglMakeCurrent failed" /tmp/pc_native.log | head -5
