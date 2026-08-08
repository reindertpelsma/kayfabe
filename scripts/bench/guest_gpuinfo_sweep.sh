#!/usr/bin/env bash
# POST_CAPTURE_HOOK — ★★★ §14.28's BISECT: ask `GPU_GET_INFO_V2` one index at a time,
# INSIDE THE GUEST, on libcuda's own subdevice handle.
#
# ## What question this answers
#
# §14.28 ended on one line of the guest-side `cuInit` trace: libcuda's eleven-index
# `0x20800102` comes back `status=0x56` with `out == in` and **no RPC crosses to this port**.
# Two candidates were named and deliberately not chosen between:
#
#   1. an arm of `getGpuInfos` failing before it reaches `default:` — its loop `break`s on
#      the first non-`NV_OK` and returns it for the WHOLE call
#      (`ogkm-580: subdevice_ctrl_gpu_kernel.c:566-569`);
#   2. the RM control cache replaying a negative — this id is `CACHEABLE_BY_INPUT`.
#
# ★ Asking each index ON ITS OWN discriminates them: an arm-level failure is attributable to
# exactly one index, and a cache hit cannot depend on which index was asked.
#
# ## ⊘ Why not `rmladder --gpu-info-sweep`
#
# R21 already asks these seventy questions — of the HOST, against real firmware. The question
# is about the GUEST. Running R21 in the guest would need its own client/device/subdevice
# setup, i.e. a second implementation whose fidelity nobody has checked; if it disagreed with
# libcuda we could not tell whether the machine or the harness differed. The interposer
# instead reuses **libcuda's own `hClient`/`hObject`**, on the same fd, in the same process,
# at the same instant — so any difference from R21 is a fact about the machine.
#
# ## ⚠ This run is an EXPERIMENT, not a capture
#
# It issues ~90 controls libcuda never issued. The interposer prints a `SWEEP-CONFIG` banner
# saying so into the trace itself, for the same reason `NVFAULT_*` does: a later reader must
# not be able to mistake one for the other. The sweep fires ONCE and only AFTER the observed
# call has completed and been logged, so the subject is never perturbed by the measurement.
set -uo pipefail

SELFDIR=$(cd "$(dirname "$0")" && pwd)
GSSH=$SELFDIR/gssh_nv
[ -x "$GSSH" ] || GSSH=/workspace/bench/kayfabe/scripts/bench/gssh_nv
TAG=${1:-gpuinfosweep}

# ★ The REPO copies, never the box copies. `boot_capture.sh` phase 0 records why: editing the
# tree and booting while a drifted `/workspace/bench/*` copy is what actually runs is a
# silent-no-op generator.
SRC_TRACE=$SELFDIR/../rpctrace/cuda_ioctl_trace.c
SRC_PROBE=$SELFDIR/../rpctrace/cuinit_probe.c
for f in "$SRC_TRACE" "$SRC_PROBE"; do
  [ -r "$f" ] || { echo "★ FAILED: $f is not readable"; exit 2; }
done
echo "=== sources (repo copies) ==="
sha256sum "$SRC_TRACE" "$SRC_PROBE"

echo "=== push interposer + probe into the guest ==="
$GSSH 'cat > /tmp/cuda_ioctl_trace.c' < "$SRC_TRACE"; echo PUSH_TRACE_RC=$?
$GSSH 'cat > /tmp/cuinit_probe.c'     < "$SRC_PROBE"; echo PUSH_PROBE_RC=$?

# ⚠ Exit codes DIRECTLY, never through a pipe — a `grep` verdict on a build log has already
# reported success on a red build (`gate_read_through_grep_cannot_fail`).
$GSSH 'gcc -O2 -Wall -shared -fPIC -o /tmp/cuda_ioctl_trace.so /tmp/cuda_ioctl_trace.c -ldl 2>&1; echo SO_RC=$?'
$GSSH 'gcc -O2 -o /tmp/cuinit_probe /tmp/cuinit_probe.c -ldl 2>&1; echo PROBE_RC=$?'
# ⊘ A missing .so makes LD_PRELOAD a silent no-op: the probe would run, cuInit would fail,
# and the trace would be EMPTY — which reads as "nothing happened" rather than "not measured".
$GSSH 'test -s /tmp/cuda_ioctl_trace.so; echo SO_PRESENT_RC=$?'
$GSSH 'test -x /tmp/cuinit_probe; echo PROBE_PRESENT_RC=$?'

echo "=== run cuInit under the interposer, sweep ON ==="
$GSSH 'cd /tmp && rm -f /tmp/gsweep.txt && NVSWEEP_GPUINFO=1 NVTRACE_OUT=/tmp/gsweep.txt NVTRACE_MAX=64 LD_PRELOAD=/tmp/cuda_ioctl_trace.so timeout 240 /tmp/cuinit_probe 2>&1; echo PROBE_EXIT=$?'

echo "=== trace size ==="
$GSSH 'wc -l /tmp/gsweep.txt 2>&1'
echo "=== ★ the sweep, in full ==="
$GSSH 'sed -n "/^SWEEP/,\$p" /tmp/gsweep.txt 2>&1'
echo "=== the non-sweep traffic (context for the sweep) ==="
$GSSH 'grep -v "^SWEEP" /tmp/gsweep.txt 2>&1'
