#!/usr/bin/env bash
# POST_CAPTURE_HOOK: isolate cuInit's OWN kernel output.
#
# ★★★ Why: boot `st1442` measured that the last thing libcuda does before tearing down is
#   `ioctl(9, UVM_IOCTL_BASE(37))` = **UVM_REGISTER_GPU** on /dev/nvidia-uvm — and
#   `uvm_gpu.c:1615` shows that call runs `uvm_channel_manager_create`, i.e. UVM allocates
#   its OWN TSG + GPFIFO channel + CE object inside cuInit. Those are `GSP_RM_ALLOC`s that
#   reach us. Every instrument this port owns was blind to it: the LD_PRELOAD interposer
#   gates on `_IOC_TYPE == 'F'` and UVM's magic is 0.
#
# ⊘ `dmesg` is CLEARED immediately before cup2 so what is printed after is cuInit's alone —
#   the device-open output (RC watchdog, CeUtils scrubber) otherwise drowns it and has
#   already been misread as cuInit's once.
set -uo pipefail
G=/workspace/bench/kayfabe/scripts/bench/gssh_nv
die() { echo "★ uvm_hook FAILED: $*"; exit 2; }

$G 'cat > /tmp/cup2.c' < /workspace/bench/cup2.c || die "could not push cup2.c"
$G 'gcc -O0 -o /tmp/cup2 /tmp/cup2.c -lcuda 2>&1; echo GCC_RC=$?'
$G 'test -x /tmp/cup2' || die "cup2 did not build"

echo "=== clear the ring buffer, then run cup2 ==="
$G 'sudo dmesg -C && echo CLEARED'
$G 'timeout 120 /tmp/cup2 2>&1; echo CUP2_RC=$?'
echo "=== dmesg produced BY cup2 ALONE ==="
$G 'sudo dmesg' 
echo "=== nvidia-uvm module state ==="
$G 'lsmod | grep uvm; cat /proc/driver/nvidia-uvm/* 2>/dev/null | head -20'
