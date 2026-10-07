#!/usr/bin/env bash
# ★ POST_CAPTURE_HOOK — the GUEST half of the per-version ioctl differential
# (`docs/design/V3_DRIVER_MATRIX.md` §5, "non-580 guests"): `cup2` run under the SAME nvdiff shim
# the host walk runs on bare metal (`tools/cudart_probe/nvdiff_shim.c`), so a guest version whose
# `cuInit` fails on kayfabe is diffed against what that version's libcuda asks a real GSP —
# `archive/nvkvm/tests/mode2/nvdiff/nvdiff.py diff <bare>.jsonl <guest>.jsonl`.
#
#   usage: KF_GUEST_IMG=<fat image> KF3_DEV_EXTRA=guest-driver=<v> KF_DEVICE=kf3 \
#          POST_CAPTURE_HOOK=scripts/drivermatrix/nvdiff_cup2_hook.sh NVD_OUT=<out>.jsonl \
#          bash scripts/bench/boot_capture.sh <tag>
#
# ⊘ The trace is appended record by record (O_APPEND, one write per record), so it is pulled
# whether or not `cup2` returned — a hang's partial trace is the measurement.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
G=${KAYFABE_GSSH:-$ROOT/scripts/bench/gssh_nv}
OUT=${NVD_OUT:-/workspace/bench/nvdiff_guest.jsonl}
DEADLINE=${NVD_DEADLINE:-120}
SHIM=$ROOT/tools/cudart_probe/nvdiff_shim.c
CUP2=$ROOT/archive/nvkvm/tests/mode2/cup2.c
for f in "$SHIM" "$CUP2" "$ROOT/scripts/bench/cuda_min/cuda.h"; do
  [ -f "$f" ] || { echo "NVD_HOOK_FAILED missing $f"; exit 2; }
done
echo "=== nvdiff: sources  shim md5 $(md5sum < "$SHIM" | cut -c1-12)  cup2 md5 $(md5sum < "$CUP2" | cut -c1-12)"
$G 'rm -rf /tmp/nvd && mkdir -p /tmp/nvd'
$G 'cat > /tmp/nvd/nvdiff_shim.c' < "$SHIM"
$G 'cat > /tmp/nvd/cup2.c' < "$CUP2"
$G 'cat > /tmp/nvd/cuda.h' < "$ROOT/scripts/bench/cuda_min/cuda.h"
# ⊘ Built exactly as the host walk builds the bare-metal half (the stand-in cuda.h, -O0), so the
# two binaries differ only in the libcuda they load.
$G 'cd /tmp/nvd && gcc -O2 -shared -fPIC -o nvdiff_shim.so nvdiff_shim.c -ldl -lpthread 2>&1 | tail -3
    L=-lcuda; [ -e /usr/lib/x86_64-linux-gnu/libcuda.so ] || L=/usr/lib/x86_64-linux-gnu/libcuda.so.1
    gcc -O0 -I/tmp/nvd -o cup2 cup2.c $L 2>&1 | tail -3
    test -x cup2 && test -f nvdiff_shim.so && echo NVD_BUILT || echo NVD_BUILD_FAILED'
$G 'cd /tmp/nvd && rm -f g.jsonl cup2.out cup2.rc
    setsid sh -c "NVDIFF_OUT=/tmp/nvd/g.jsonl LD_PRELOAD=/tmp/nvd/nvdiff_shim.so ./cup2 > /tmp/nvd/cup2.out 2>&1; echo \$? > /tmp/nvd/cup2.rc" </dev/null >/dev/null 2>&1 &
    sleep 1; echo LAUNCHED'
done=0
for _ in $(seq 1 $((DEADLINE / 5))); do
  $G 'test -f /tmp/nvd/cup2.rc' 2>/dev/null && { done=1; break; }
  sleep 5
done
[ "$done" = 1 ] || echo "★ cup2 did not return within ${DEADLINE}s — the trace below is partial by construction"
$G 'cat /tmp/nvd/cup2.out; echo "CUP2_RC=$(cat /tmp/nvd/cup2.rc 2>/dev/null || echo TIMEOUT)"'
$G 'cat /tmp/nvd/g.jsonl' > "$OUT"
n=$(wc -l < "$OUT" 2>/dev/null || echo 0)
# ⊘ An empty trace is not "the guest issued nothing" — it is the shim not attaching or the pull
# failing, and it must never be diffed as if it were a measurement.
if [ "$n" -lt 50 ]; then echo "NVD_TRACE_EMPTY records=$n → $OUT"; exit 3; fi
echo "NVD_TRACE records=$n → $OUT"
