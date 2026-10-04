#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Dedicated, disposable 575.51.03 fixture only. Artifacts remain on its controller.
set -euo pipefail
cd /opt/kf-unpriv
cc -O2 -Wall -Wextra -Werror -I/root/ogkm-575.51.03/src/common/sdk/nvidia/inc -I/root/ogkm-575.51.03/src/common/inc -o read_queries read_queries.c
for m in nvidia_drm nvidia_modeset nvidia_uvm nvidia; do
    if test -d /sys/module/"$m"; then modprobe -r "$m"; fi
done
insmod /root/kf-unpriv-instrumented/nvidia.ko NVreg_EnableGpuFirmware=1 NVreg_RpcTraceKB=131072
insmod /root/kf-unpriv-instrumented-build/kernel-open/nvidia-uvm.ko
# Stream credential stamps: a final dmesg snapshot can lose early sends when
# the kernel's printk ring wraps, even though the RPC ring remains complete.
dmesg --follow-new > evidence/instrumented-kernel.log &
kernel_log=$!
trap 'kill "$kernel_log" 2>/dev/null || true; wait "$kernel_log" 2>/dev/null || true' EXIT
sleep 0.2
kill -0 "$kernel_log"
cat /proc/driver/nvidia/version > evidence/instrumented-driver.txt
sha256sum /root/kf-unpriv-instrumented/nvidia.ko > evidence/instrumented-module-sha256.txt
nvidia-smi -q > evidence/instrumented-gpu-root-inventory.txt
BPFTRACE_PERF_RB_PAGES=256 bpftrace -B line trace.bt > evidence/open-ioctl-events.log 2> evidence/open-ioctl-events.err &
tracer=$!
trap 'kill -INT "$tracer" 2>/dev/null || true; wait "$tracer" 2>/dev/null || true; kill "$kernel_log" 2>/dev/null || true; wait "$kernel_log" 2>/dev/null || true' EXIT
for i in $(seq 1 50); do
    kill -0 "$tracer"
    if grep -q TRACE_HEARTBEAT evidence/open-ioctl-events.log; then break; fi
    sleep 0.1
done
grep -q TRACE_HEARTBEAT evidence/open-ioctl-events.log
for test in smi cuda vector queries; do
    case "$test" in
      smi) set -- nvidia-smi -q;;
      cuda) set -- /opt/kf-unpriv/cuinit_probe;;
      vector) set -- /opt/kf-unpriv/vector_add_test;;
      queries) set -- /opt/kf-unpriv/read_queries;;
    esac
    setpriv --reuid=65534 --regid=65534 --clear-groups --bounding-set=-all --inh-caps=-all --ambient-caps=-all --no-new-privs env AUDIT_PRELOAD=/opt/kf-unpriv/nvdiff.so NVDIFF_OUT=/opt/kf-unpriv/evidence/open-${test}-ioctl.jsonl /opt/kf-unpriv/exec_unprivileged "$@" > evidence/open-${test}.out 2> evidence/open-${test}.posture
    echo "OPEN_TEST $test rc=$?"
done
kill -INT "$tracer"
wait "$tracer"
trap 'kill "$kernel_log" 2>/dev/null || true; wait "$kernel_log" 2>/dev/null || true' EXIT
cat /proc/driver/nvidia/rpctrace > /root/kf-unpriv-native.bin
python3 export_native.py /root/kf-unpriv-native.bin evidence/native-gsp.jsonl.gz --decoder-dir /root/rpctrace > evidence/native-gsp-manifest.json
kill "$kernel_log"
wait "$kernel_log" || true
trap - EXIT
sha256sum *.c *.h *.bt *.sh *.py credential-stamp.patch > evidence/final-input-sha256.txt
echo CAPTURE_COMPLETE
