#!/usr/bin/env bash
# nvd_capture.sh — build the shim + workload and record N runs.
#
# Identical on the real host and inside a Mode-2 guest; that symmetry is the
# whole point (one instrument, two subjects).
#
#   ./nvd_capture.sh <outdir> [stage] [runs]
#     stage : init|dev|ctx|alloc|ce|launch   (default ce == the cup2 shape)
#     runs  : how many repetitions          (default 2, which gives a noise floor)
#
# Produces  <outdir>/<stage>_r<N>.jsonl  plus  <outdir>/<stage>_r<N>.stdout
# and asserts each capture is NON-EMPTY and contains at least one RM_CONTROL --
# a harness that writes an empty file and exits 0 is worse than none.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="${1:?usage: nvd_capture.sh <outdir> [stage] [runs]}"
STAGE="${2:-ce}"
RUNS="${3:-2}"
mkdir -p "$OUT"

CC="${CC:-cc}"
CFLAGS_CUDA="${CFLAGS_CUDA:-}"
: "${CUDA_INC:=}"
# ⊘ NVD_MIN_CUDA=1 forces the bundled minimal header even where a real cuda.h exists.
#   ★ USE IT ON BOTH SIDES OR NEITHER. The differential's whole validity is that host and
#   guest run the SAME program; building one side against the toolkit header and the other
#   against the stand-in makes the two binaries a variable, and it is the variable nobody
#   would think to look at.
if [ "${NVD_MIN_CUDA:-0}" != 1 ]; then
    for d in /usr/local/cuda/include /usr/include /usr/local/include; do
        # ⚠ a file named cuda.h is not cuda.h: the PowerMac ADB driver header has the same
        #   name and is present on both the bench box and the guest. Check the content.
        [ -f "$d/cuda.h" ] && grep -q 'CUDA_SUCCESS' "$d/cuda.h" 2>/dev/null && CUDA_INC="$d" && break
    done
fi
if [ -n "$CUDA_INC" ]; then
    echo "== build (REAL cuda.h from $CUDA_INC)"
    INCFLAG="-I$CUDA_INC"
else
    echo "== build (⊘ no real cuda.h — using the bundled nvd_cuda_min.h stand-in)"
    INCFLAG="-DNVD_NO_CUDA_H -I$HERE"
fi
$CC -shared -fPIC -O2 -o "$OUT/nvdiff_shim.so" "$HERE/nvdiff_shim.c" -ldl -lpthread || exit 1
$CC -O0 $INCFLAG -o "$OUT/nvd_prog" "$HERE/nvd_prog.c" -lcuda $CFLAGS_CUDA || {
    echo "FATAL: could not link -lcuda (need libcuda.so dev symlink)"; exit 1; }

# ★★★ SYMBOL-BINDING GATE. A header cannot check itself; the linker's output can.
# Real cuda.h #defines seven entry points onto their _v2 symbols. Binding the v1 names
# instead builds, links, runs, and emits a DIFFERENT ioctl stream — silently. Refuse.
echo "== symbol-binding gate (the seven versioned entry points must bind _v2)"
MISSING=
# ⚠ cuMemcpyDtoD_v2 joined the list with the fault stages (2026-08-12). It is compiled
#   unconditionally, so the gate applies to every build, not only to a fault capture.
for sym in cuDeviceTotalMem_v2 cuCtxCreate_v2 cuCtxDestroy_v2 cuMemAlloc_v2 \
           cuMemFree_v2 cuMemcpyHtoD_v2 cuMemcpyDtoH_v2 cuMemcpyDtoD_v2; do
    # ⚠ objdump prints `cuCtxCreate_v2@Base` (or `@LIBCUDA_1.0`), never a bare name — an
    #   end-anchored match fails on a CORRECTLY bound symbol. Measured: the first version of
    #   this gate refused a build in which all seven were bound. It failed SAFE, which is the
    #   only reason it cost minutes and not a wrong reference capture.
    if objdump -R "$OUT/nvd_prog" 2>/dev/null | grep -qE "[[:space:]]$sym(@|$)"; then
        echo "   ok   $sym"
    else
        echo "   ★★★ NOT BOUND: $sym"; MISSING="$MISSING $sym"
    fi
done
[ -z "$MISSING" ] || { echo "FATAL: v1 symbols bound instead of _v2 —$MISSING"; exit 1; }

echo "== environment"
{
  echo "date=$(date -u +%FT%TZ)"
  echo "uname=$(uname -a)"
  echo "driver=$(cat /proc/driver/nvidia/version 2>/dev/null | head -1)"
  echo "devnodes=$(ls -1 /dev/nvidia* 2>/dev/null | tr '\n' ' ')"
  echo "libcuda=$(ldconfig -p 2>/dev/null | grep -m1 libcuda.so.1 || echo none)"
  echo "stage=$STAGE runs=$RUNS"
  echo "shim_md5=$(md5sum "$HERE/nvdiff_shim.c" | cut -d' ' -f1)"
  echo "prog_md5=$(md5sum "$HERE/nvd_prog.c" | cut -d' ' -f1)"
} | tee "$OUT/env_$STAGE.txt"

rc_all=0
# ⊘ runs=0 is BUILD-ONLY, and it exists because a caller that wants to control the run
#   itself (detached, with its own timeout, extracting the trace mid-hang) would otherwise
#   have to run the workload once just to get a binary — on a GPU that is a serial resource.
if [ "$RUNS" -eq 0 ]; then
    echo "== runs=0: BUILD ONLY, nothing was executed (this is not an empty capture)"
    exit 0
fi
for i in $(seq 1 "$RUNS"); do
    f="$OUT/${STAGE}_r${i}.jsonl"
    rm -f "$f"
    echo "== run $i -> $f"
    NVDIFF_OUT="$f" NVDIFF_MAXBUF="${NVDIFF_MAXBUF:-65536}" \
        LD_PRELOAD="$OUT/nvdiff_shim.so" \
        timeout "${NVD_TIMEOUT:-300}" "$OUT/nvd_prog" "$STAGE" \
        > "$OUT/${STAGE}_r${i}.stdout" 2>&1
    rc=$?
    echo "   prog rc=$rc  records=$(wc -l < "$f" 2>/dev/null || echo 0)"
    tail -3 "$OUT/${STAGE}_r${i}.stdout" | sed 's/^/   | /'
    # ASSERT the capture is real. An existing file is not a capture.
    if [ ! -s "$f" ]; then
        echo "   FATAL: capture is EMPTY -- the shim did not attach"; rc_all=1; continue
    fi
    if ! grep -q '"nr":42' "$f"; then
        echo "   FATAL: no RM_CONTROL (nr=0x2a) in capture -- not a real RM stream"; rc_all=1
    fi
    [ $rc -ne 0 ] && rc_all=$rc
done
echo "== done rc=$rc_all"
exit $rc_all
