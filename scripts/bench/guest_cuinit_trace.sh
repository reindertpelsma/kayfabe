#!/usr/bin/env bash
# POST_CAPTURE_HOOK — ★★★ run the §14.27 INTERPOSER INSIDE THE GUEST.
#
# ⊘ Why this and not more injection: single-fault injection subtracts one answer from a
# system that WORKS. It found `0x2081` because that is a singleton necessary cause. Driven
# over every id this port still refuses (16 of them, §14.28), it names NOTHING — because it
# runs on REAL hardware, where everything else is answered correctly. It is structurally
# blind both to a CONJUNCTION and to an answer that is SERVED BUT WRONG.
#
# ★ This points the same instrument at OUR port, so the output is directly diffable against
# `traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt` — a comparison, not a subtraction.
set -uo pipefail
GSSH=/workspace/bench/kayfabe/scripts/bench/gssh_nv
TAG=${1:-guesttrace}
echo "=== push the interposer + probe into the guest ==="
$GSSH 'cat > /tmp/cuda_ioctl_trace.c' < /workspace/bench/cuda_ioctl_trace.c
$GSSH 'cat > /tmp/cuinit_probe.c'     < /workspace/bench/cuinit_probe.c
$GSSH 'gcc -O0 -shared -fPIC -o /tmp/cuda_ioctl_trace.so /tmp/cuda_ioctl_trace.c -ldl 2>&1; echo SO_RC=$?'
$GSSH 'gcc -O0 -o /tmp/cuinit_probe /tmp/cuinit_probe.c -ldl 2>&1; echo PROBE_RC=$?'
echo "=== run cuInit under the interposer ==="
$GSSH 'cd /tmp && rm -f /tmp/gtrace.txt && LD_PRELOAD=/tmp/cuda_ioctl_trace.so NVTRACE_FILE=/tmp/gtrace.txt timeout 180 /tmp/cuinit_probe 2>&1; echo PROBE_EXIT=$?'
echo "=== trace size ==="
$GSSH 'wc -l /tmp/gtrace.txt 2>&1'
echo "=== the whole trace ==="
$GSSH 'cat /tmp/gtrace.txt 2>&1'
