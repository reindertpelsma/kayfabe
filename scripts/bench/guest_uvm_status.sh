#!/usr/bin/env bash
# POST_CAPTURE_HOOK: read `UVM_REGISTER_GPU`'s `params.rmStatus` — the value that decides
# `cuInit` and that no instrument in this project has ever printed.
#
# ★★★ §14.39 measured that `cuInit`'s last productive syscall is
# `ioctl(9, UVM_IOCTL_BASE(37))` on `/dev/nvidia-uvm`, and that it returns **0**. UVM answers
# 0 at the syscall boundary for every call it dispatched at all and puts the verdict in
# `params.rmStatus` (`ogkm-580: uvm_ioctl.h:534-543`). ⊘ A `strace` line reading `= 0` is
# therefore not evidence the call succeeded — it is evidence the call was dispatched.
# `uvm_ioctl_trace.so` reads the struct after the call and prints that field.
#
# ⊘ `dmesg` is cleared immediately before `cup2` so what follows is `cuInit`'s alone: the
# device-open output (RC watchdog, CeUtils scrubber) otherwise drowns it and has already been
# misread as `cuInit`'s once. ★ And the driver is `modprobe`d over ssh, so its output goes to
# whoever ran the command and NOWHERE ELSE — the serial log does not have it. Everything here
# is written back to the probe log, and the capture is asserted non-empty before it is cited.
set -uo pipefail
REPO=/workspace/bench/kayfabe
G="$REPO/scripts/bench/gssh_nv"
die() { echo "★ uvm_status hook FAILED: $*"; exit 2; }

$G 'cat > /tmp/cup2.c' < /workspace/bench/cup2.c              || die "could not push cup2.c"
$G 'cat > /tmp/uvm_ioctl_trace.c' < "$REPO/scripts/bench/uvm_ioctl_trace.c" \
                                                              || die "could not push the interposer"
$G 'gcc -O0 -o /tmp/cup2 /tmp/cup2.c -lcuda 2>&1; echo GCC_CUP2_RC=$?'
$G 'gcc -O1 -fPIC -shared -o /tmp/uvm_ioctl_trace.so /tmp/uvm_ioctl_trace.c -ldl 2>&1; echo GCC_SHIM_RC=$?'
$G 'test -x /tmp/cup2 && test -s /tmp/uvm_ioctl_trace.so'     || die "cup2 or the interposer did not build"

echo "=== clear the ring buffer, then run cup2 UNDER the UVM interposer ==="
$G 'sudo dmesg -C && echo CLEARED'
$G 'rm -f /tmp/uvm.txt; UVMTRACE_OUT=/tmp/uvm.txt LD_PRELOAD=/tmp/uvm_ioctl_trace.so timeout 180 /tmp/cup2 2>&1; echo CUP2_RC=$?'

echo "=== ★★★ THE UVM PLANE (this is the measurement) ==="
$G 'cat /tmp/uvm.txt 2>&1; echo UVM_LINES=$(wc -l < /tmp/uvm.txt 2>/dev/null || echo 0)'

echo "=== the rmStatus lines alone ==="
$G 'grep -a "rmStatus" /tmp/uvm.txt 2>&1 || echo "NO rmStatus LINE — the interposer saw no UVM ioctl at all"'

echo "=== dmesg produced BY cup2 ALONE ==="
$G 'sudo dmesg'

echo "=== anything UVM printed for itself (UVM_ERR_PRINT lands here) ==="
$G 'sudo dmesg | grep -ai "uvm" || echo "NO UVM LINE IN dmesg"'

echo "=== nvidia-uvm module state ==="
$G 'lsmod | grep -i uvm; ls -la /dev/nvidia-uvm*'
