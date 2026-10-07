#!/usr/bin/env bash
# tools/uvm_efs/box/run_experiment.sh — the b3 host-only experiment, on the box.
#   bash run_experiment.sh <patched-ko-dir> [efs-src-dir]
# Loads the patched nvidia-uvm with uvm_efs_enable=1, builds the three test binaries against the
# pinned sources, and runs the full matrix. Writes /root/efs/run.log with a START marker, a tag
# per step and an EXIT line (CLAUDE.md trap: an empty file is not "not yet"). Loads the STOCK
# module back at the end. Every test's own RESULT line is the verdict; the orchestrator greps them.
set -uo pipefail
KO_DIR=${1:-/root/efs/patched}
SRC=${2:-/root/kayfabe/tools/uvm_efs}
OGKM=/root/ogkm
UVM_SRC=$KO_DIR/nvidia-uvm
mkdir -p /root/efs
exec >>/root/efs/run.log 2>&1
echo "RUN_START $(date -Is) ko_dir=$KO_DIR"

INC="-I$OGKM/src/common/sdk/nvidia/inc -I$OGKM/src/common/sdk/nvidia/inc/class -I$UVM_SRC -I$OGKM/kernel-open/common/inc"
NVCC=/usr/local/cuda-12.6/bin/nvcc

echo "=== build tests ==="
$NVCC -cubin -arch=sm_86 -o /root/efs/efs_kernels.cubin "$SRC/tests/efs_kernels.cu" 2>&1 | tail -5
echo "CUBIN_RC=$? $(ls -l /root/efs/efs_kernels.cubin 2>&1 | awk '{print $5}')"
g++ -O2 -std=c++14 -o /root/efs/efs_fault "$SRC/tests/efs_fault.cpp" $INC -I/usr/local/cuda-12.6/include \
    -L/usr/local/cuda-12.6/lib64/stubs -lcuda -lpthread 2>&1 | tail -20
echo "FAULT_RC=$?"
g++ -O2 -std=c++14 -o /root/efs/efs_auth "$SRC/tests/efs_auth.cpp" $INC 2>&1 | tail -20
echo "AUTH_RC=$?"
$NVCC -O2 -arch=sm_86 -o /root/efs/coexist "$SRC/tests/coexist.cu" 2>&1 | tail -10
echo "COEXIST_RC=$?"
export EFS_CUBIN=/root/efs/efs_kernels.cubin
[ -x /root/efs/efs_fault ] && [ -x /root/efs/efs_auth ] && [ -x /root/efs/coexist ] || { echo "BUILD_INCOMPLETE"; echo "EXIT rc=10 $(date -Is)"; exit 10; }

load_efs() { rmmod nvidia_uvm 2>/dev/null; insmod "$KO_DIR/nvidia-uvm.ko" uvm_efs_enable="${1:-1}" "${@:2}"; echo "insmod_rc=$? $(head -1 /proc/driver/nvidia/version)"; nvidia-smi -L | head -1; }
load_stock() { rmmod nvidia_uvm 2>/dev/null; modprobe nvidia_uvm; echo "stock_modprobe_rc=$? efs_present=$(grep -c uvm_efs_enable /sys/module/nvidia_uvm/parameters/* 2>/dev/null || echo 0)"; }
run() { echo "### $* $(date -Is)"; /root/efs/"$@"; echo "### exit=$? $*"; echo; }

echo "=== A. stock module: EFS absent, unmapped access must fail (baseline) ==="
load_stock
run efs_fault stock 1

echo "=== B. patched module, default OFF: stock behaviour, EFS refused ==="
load_efs 0
run efs_auth
run efs_fault stock 1

echo "=== C. patched module, EFS ENABLED ==="
load_efs 1 uvm_efs_timeout_ms=4000
run efs_auth
echo "--- C1 negative control: pre-mapped, zero faults ---"
run efs_fault negative 8
echo "--- C2 service: real shader fault -> deliver -> map (cuMemMap) -> replay -> correct data ---"
run efs_fault service 32
echo "--- C3 service, kayfabe shape: map with our own RM vidmem via UVM ioctls ---"
run efs_fault service 8 raw
echo "--- C4 read faults ---"
run efs_fault read 16
echo "--- C5 scoped cancel: the faulting VA space fails, nothing else ---"
run efs_fault cancel
echo "--- C6 kernel timeout cancels a never-answered fault ---"
run efs_fault timeout
echo "--- C7 crash with a fault parked (VMM death) ---"
run efs_fault crash 50 ; echo "crash_shell_exit=$?"
sleep 2; nvidia-smi -L | head -1 ; echo "smi_after_crash=$?"
echo "--- C8 context destroy with a fault pending ---"
run efs_fault ctxdestroy

echo "=== D. COEXISTENCE: native host CUDA (matmul + managed demand paging) + EFS fault pressure ==="
load_efs 1 uvm_efs_timeout_ms=4000
echo "--- D0 coexist alone (baseline throughput) ---"
run coexist 6
echo "--- D1 coexist WHILE an EFS VA space repeatedly faults+services ---"
/root/efs/coexist 12 > /root/efs/coexist_during.log 2>&1 &
COEX=$!
sleep 1
for i in 1 2 3 4 5; do /root/efs/efs_fault service 32 > /root/efs/efs_during_$i.log 2>&1; echo "efs_during_$i exit=$?"; done
wait $COEX; echo "coexist_during exit=$?"
echo "COEXIST_DURING:"; grep -aE "THROUGHPUT|RESULT|CUDA_FAIL" /root/efs/coexist_during.log
for i in 1 2 3 4 5; do echo "EFS_DURING_$i:"; grep -aE "^RESULT|DATA bad|RECORDS|DELIVERY" /root/efs/efs_during_$i.log; done
echo "--- D2 coexist WHILE an EFS VA space parks a fault for 3s (a slow VMM) ---"
/root/efs/coexist 8 > /root/efs/coexist_park.log 2>&1 &
COEX=$!
/root/efs/efs_fault park 3000 > /root/efs/efs_park.log 2>&1; echo "efs_park exit=$?"
wait $COEX; echo "coexist_park exit=$?"
echo "COEXIST_PARK:"; grep -aE "THROUGHPUT|RESULT" /root/efs/coexist_park.log
echo "EFS_PARK:"; grep -aE "^RESULT|PARK_|DELIVERY|DATA bad" /root/efs/efs_park.log

echo "=== E. latency (median/p99), quiet box, larger sample ==="
load_efs 1 uvm_efs_timeout_ms=10000
run efs_fault service 256

echo "=== restore stock module ==="
load_stock
echo "RUN_EXIT $(date -Is)"
