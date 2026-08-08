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
#
# ## ⊘⊘ THE "WHICH COPY RUNS" TRAP, FOUND AGAIN AT §14.31 — this time on the PAYLOAD
#
# `boot_capture.sh` documents this trap at length and fixes it for the *helpers* (`BOOTER`,
# `GSSH`). This script pushed `/workspace/bench/cuda_ioctl_trace.c` — the **box** copy —
# while the repository carries `scripts/rpctrace/cuda_ioctl_trace.c`.
# `[measured 2026-08-08]` they are DIFFERENT files: the box copy is 445 lines to the repo copy's 666, and has
# no `NVSWEEP_GPUINFO` block at all, so §14.29's in-guest bisect mode has **never been
# reachable through this hook**. Editing the tree, pushing and booting ran the other file.
# ⇒ The repo copy wins below, and the source is printed so a run cannot be silently the
# other one. `KAYFABE_TRACE_SRC` overrides it for a box that genuinely needs a variant.
set -uo pipefail
SELFDIR=$(cd "$(dirname "$0")" && pwd)
GSSH="$SELFDIR/gssh_nv"
TRACE_SRC=${KAYFABE_TRACE_SRC:-$SELFDIR/../rpctrace/cuda_ioctl_trace.c}
[ -f "$TRACE_SRC" ] || { echo "★ no interposer source at $TRACE_SRC"; exit 2; }
TAG=${1:-guesttrace}
echo "=== interposer source: $TRACE_SRC ($(wc -l < "$TRACE_SRC") lines, md5 $(md5sum < "$TRACE_SRC" | cut -d' ' -f1)) ==="
echo "=== push the interposer + probe into the guest ==="
$GSSH 'cat > /tmp/cuda_ioctl_trace.c' < "$TRACE_SRC"
$GSSH 'cat > /tmp/cuinit_probe.c'     < /workspace/bench/cuinit_probe.c
$GSSH 'gcc -O0 -shared -fPIC -o /tmp/cuda_ioctl_trace.so /tmp/cuda_ioctl_trace.c -ldl 2>&1; echo SO_RC=$?'
$GSSH 'gcc -O0 -o /tmp/cuinit_probe /tmp/cuinit_probe.c -ldl 2>&1; echo PROBE_RC=$?'
echo "=== run cuInit under the interposer ==="
# ⊘⊘ `NVTRACE_FILE` was a MISNAME — the interposer reads `NVTRACE_OUT` (`trace_init`), so
# this line wrote NO file and the whole trace reached us only as the ssh session's stderr.
# `[measured 2026-08-08]` The two commands below it (`wc -l`, `cat`) were therefore reading a
# path that never existed, and the trace §14.28 is built on lives in a transcript rather than
# on the box — the serial-log trap (`CLAUDE.md`, trap 1) reproduced by a typo in an env var.
$GSSH 'cd /tmp && rm -f /tmp/gtrace.txt && LD_PRELOAD=/tmp/cuda_ioctl_trace.so NVTRACE_OUT=/tmp/gtrace.txt timeout 180 /tmp/cuinit_probe 2>&1; echo PROBE_EXIT=$?'
echo "=== trace size ==="
$GSSH 'wc -l /tmp/gtrace.txt 2>&1'
echo "=== the whole trace ==="
$GSSH 'cat /tmp/gtrace.txt 2>&1'
# ⚠⚠ `[measured 2026-08-08, boot gt1431_ff7a0ea]` /tmp/gtrace.txt was **ABSENT** after the
# run even though `NVTRACE_OUT` was set — the two commands above printed
# "No such file or directory" and the trace reached us only as the interposer's stderr,
# which the caller's probe log happened to capture. ⊘ That is the §14.29 defect surviving
# its own fix: the env var is right, and the file still is not there.
# ⇒ Say so loudly rather than exit 0 over a missing artifact. The run is still usable,
# because the probe log IS a file on disk — but a caller must know which one it is reading.
$GSSH 'test -s /tmp/gtrace.txt' \
  || echo "★ TRACE ARTIFACT MISSING: /tmp/gtrace.txt is absent or empty. The trace above is
★   the interposer's STDERR as captured by whoever ran this hook — persist THAT, and do not
★   record this run as having produced a trace file."
