#!/usr/bin/env bash
# ★★★★★ THE CUDA LADDER ON kf3 — cup2 → cup3 → cup8 → cup8bench, guest arm and host arm.
#
# > Owner, 2026-09-25: *"Test the CUDA ladder. How much slower is the raw client compared to
# > host now."* — and, the same day: *"run the CUDA ladder in the FAT guest (the full qcow2 OS —
# > gcc + libcuda, the old boot_capture.sh path) booted with the CURRENT kayfabe v3 device …
# > keep POST_CAPTURE_HOOK so cup2/cup3/cup8/cup8bench hooks run unchanged."*
#
# ## guest arm — the fat guest, the ladder's own hooks, unchanged
#   one `boot_capture.sh` boot per rung per rep with `KF_DEVICE=kf3` (boot_nvkvm.sh picks the
#   per-revision kf3 binary, memfd guest RAM, `-device kf3-gpu`) and the rung's hook:
#     cup2       cup2_hook_deadline.sh   graded `CE rv=0xabcd1234 … -> PASS`
#     cup3       cup3_hook.sh            graded `^CUP3_VAL=43`
#     cup8       cup8_hook.sh            graded `^CUP8_BAD=0` `^CUP8_MAXERR=0`
#     cup8bench  cup8bench_hook.sh       graded `GUEST_BENCH_TOTAL_BAD=0` / `BENCH_VERDICT: PASS`
#   plus the device's per-token DOORBELL-LEDGER from the boot's qemu log (forwarded>0 per guest
#   token that rang; emulated=0), and the device's own trapped-exit count (`kf3: … trapped=N`).
#   One rung per boot: the ledger then belongs to exactly that workload.
#
# ## host arm — the same sources on bare metal, same box
#   The box has libcuda but no toolkit, so cup2/cup3/cup8 are compiled against
#   `scripts/bench/cuda_min/cuda.h` (the dozen Driver-API entry points they use; cup8bench.c
#   declares its own and is built with the hook's exact gcc line). Graded on the same values.
#
# usage: cuda_ladder.sh host  <tag> [reps=3] [rungs=cup2,cup3,cup8,cup8bench]
#        cuda_ladder.sh guest <tag> [reps=3] [rungs=cup2,cup3,cup8,cup8bench]
# env:   KAYFABE_BENCH_SIZES (default 16,2048: N=16 isolates per-launch overhead, N=2048 is cup8)
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
BENCH=${BENCH_DIR:-/workspace/bench}
BINS=$BENCH/cudaladder-host-bins
CUP2_C=$REPO/archive/nvkvm/tests/mode2/cup2.c
export KAYFABE_BENCH_SIZES=${KAYFABE_BENCH_SIZES:-16,2048}
B_ITERS=${KAYFABE_BENCH_ITERS:-20} B_BATCH=${KAYFABE_BENCH_BATCH:-10}
die() { echo "cuda_ladder: $*" >&2; exit 2; }
REV=$(git -C "$REPO" rev-parse --short=8 HEAD)

grade() {  # $1 rung, $2 file → prints graded lines; rc 0 iff graded PASS
    local r=$1 f=$2
    case "$r" in
        cup2) grep -a 'CE rv=' "$f" | tail -1; grep -aq 'CE rv=0xabcd1234 want=0xabcd1234 -> PASS' "$f" ;;
        cup3) grep -a '^CUP3_VAL=' "$f" | tail -1; grep -aq '^CUP3_VAL=43$' "$f" ;;
        cup8) grep -a '^CUP8_BAD=\|^CUP8_MAXERR=' "$f" | tail -2
              grep -aq '^CUP8_BAD=0$' "$f" && grep -aq '^CUP8_MAXERR=0$' "$f" ;;
        # host prints `BENCH_VERDICT: PASS`; the guest hook lifts it to `GUEST_BENCH_VERDICT=PASS`
        cup8bench) grep -a '^BENCH_VERDICT:\|^BENCH_TOTAL_BAD=\|^GUEST_BENCH_VERDICT=\|^GUEST_BENCH_TOTAL_BAD=' "$f" | tail -2
              { grep -aq '^BENCH_VERDICT: PASS' "$f" && grep -aq '^BENCH_TOTAL_BAD=0$' "$f"; } \
              || { grep -aq '^GUEST_BENCH_VERDICT=PASS' "$f" && grep -aq '^GUEST_BENCH_TOTAL_BAD=0$' "$f"; } ;;
    esac
}

MODE=${1:-}; TAG=${2:-}; REPS=${3:-3}; RUNGS=${4:-cup2,cup3,cup8,cup8bench}
[ -n "$TAG" ] || { sed -n '2,32p' "$0"; exit 2; }
OUT=$BENCH/cl_${TAG}_${MODE}.out
{ echo "CUDA_LADDER_STARTED=$(date -Is) mode=$MODE reps=$REPS rungs=$RUNGS rev=$REV"
  echo "bench sizes=$KAYFABE_BENCH_SIZES iters=$B_ITERS batch=$B_BATCH"
} > "$OUT"

case "$MODE" in
host)
    command -v gcc >/dev/null || die "no gcc on the host"
    L=-lcuda; [ -e /usr/lib/x86_64-linux-gnu/libcuda.so ] || L=/usr/lib/x86_64-linux-gnu/libcuda.so.1
    rm -rf "$BINS"; mkdir -p "$BINS"
    I="-I$REPO/scripts/bench/cuda_min"
    gcc -O0 $I -o "$BINS/cup2" "$CUP2_C" $L                        || die "cup2 build failed"
    gcc -O0 $I -o "$BINS/cup3" "$HERE/cup3.c" $L                   || die "cup3 build failed"
    gcc -O0 $I -o "$BINS/cup8" "$HERE/cup8.c" $L -lm               || die "cup8 build failed"
    gcc -O2    -o "$BINS/cup8bench" "$HERE/cup8bench.c" $L -lm     || die "cup8bench build failed"
    echo "host libcuda=$(readlink -f /usr/lib/x86_64-linux-gnu/libcuda.so.1)" >> "$OUT"
    for s in "$CUP2_C" "$HERE/cup3.c" "$HERE/cup8.c" "$HERE/cup8bench.c"; do
        echo "src $(basename "$s") md5 $(md5sum < "$s" | cut -d' ' -f1)" >> "$OUT"; done
    for r in $(echo "$RUNGS" | tr ',' ' '); do
        for i in $(seq 1 "$REPS"); do
            log=$BENCH/cl_${TAG}_host_${r}_$i.log
            exec 9>"${KF_LOCK:-/tmp/kayfabe-fastguest.lock}"; flock 9   # GPU runs strictly serial
            envp=""
            [ "$r" = cup8bench ] && envp="BENCH_SIZES=$KAYFABE_BENCH_SIZES BENCH_ITERS=$B_ITERS BENCH_BATCH=$B_BATCH"
            t0=$(date +%s%N)
            ( cd /tmp && env $envp timeout 900 "$BINS/$r" ) > "$log" 2>&1; prc=$?
            t1=$(date +%s%N)
            exec 9>&-
            case "$r" in
                cup3) echo "CUP3_VAL=$(sed -n 's/^KERNEL rv=\([0-9]*\) .*/\1/p' "$log" | tail -1)" >> "$log" ;;
                cup8) l=$(grep '^CUP8 RESULT ' "$log" | tail -1)
                      echo "CUP8_BAD=$(echo "$l" | sed -n 's/.* bad=\([0-9]*\) .*/\1/p')" >> "$log"
                      echo "CUP8_MAXERR=$(echo "$l" | sed -n 's/.* maxerr=\([^ ]*\) .*/\1/p')" >> "$log" ;;
            esac
            g=$(grade "$r" "$log" | tr '\n' ' '); grade "$r" "$log" >/dev/null && v=PASS || v=FAIL
            echo "CL_ROW mode=host rung=$r rep=$i verdict=$v rc=$prc wall_ms=$(( (t1 - t0) / 1000000 )) graded=[${g% }]" >> "$OUT"
            [ "$r" = cup8bench ] && grep -a '^BENCH_INIT_MS=\|^BENCH_CTX_MS=\|^BENCH_MODULE_MS=\|^BSUM ' "$log" \
                | sed "s/^/CL_BENCH mode=host rep=$i /" >> "$OUT"
            tail -1 "$OUT"
        done
    done
    ;;
guest)
    for r in $(echo "$RUNGS" | tr ',' ' '); do
        case "$r" in
            cup2) hook=$HERE/cup2_hook_deadline.sh ;;
            cup3) hook=$HERE/cup3_hook.sh ;;
            cup8) hook=$HERE/cup8_hook.sh ;;
            cup8bench) hook=$HERE/cup8bench_hook.sh ;;
            *) die "unknown rung $r" ;;
        esac
        for i in $(seq 1 "$REPS"); do
            t=cl_${TAG}_${r}_$i
            # ⊘ w826: the previous VM's isolate can hold its GPU reservation for a moment after
            # QEMU exits — wait for the GPU to drain rather than let realize refuse.
            for _w in $(seq 1 60); do
                u=$(nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits 2>/dev/null | head -1 | tr -dc 0-9)
                [ -n "$u" ] && [ "$u" -le 512 ] && break; sleep 0.5
            done
            KF_DEVICE=kf3 POST_CAPTURE_HOOK=$hook GQ_TIMEOUT=${GQ_TIMEOUT:-600} \
            CUP2_SRC=$CUP2_C KAYFABE_CUP3_TIMEOUT=${KAYFABE_CUP3_TIMEOUT:-120} \
            KAYFABE_BENCH_ONLY=${KAYFABE_BENCH_ONLY:-measure} KAYFABE_BENCH_ITERS=$B_ITERS KAYFABE_BENCH_BATCH=$B_BATCH \
                bash "$HERE/boot_capture.sh" "$t" > "$BENCH/${t}_driver.log" 2>&1
            brc=$?
            probe=$BENCH/run_${t}_probe.log; q=$BENCH/run_${t}_qemu.log
            g=$(grade "$r" "$probe" | tr '\n' ' '); grade "$r" "$probe" >/dev/null && v=PASS || v=FAIL
            led=$(grep -ao 'DOORBELL-LEDGER tok=[^ ]* .*' "$q" 2>/dev/null \
                  | sed -n 's/.*tok=\(0x[0-9a-f]*\) .*emulated=\([0-9]*\) forwarded=\([0-9]*\).*/\1:e\2f\3/p' | tr '\n' ' ')
            trapped=$(grep -ao 'kf3: family=[A-Za-z]* phase=[A-Za-z]* trapped=[0-9]*' "$q" 2>/dev/null | tail -1 | sed 's/.*trapped=//')
            wall=""
            case "$r" in
                cup8) wall=$(grep -a '^CUP8_WALL_S=' "$probe" | tail -1) ;;
                cup8bench) wall=$(grep -a '^BENCH_measure_WALL_S=' "$probe" | tail -1) ;;
            esac
            echo "CL_ROW mode=guest rung=$r rep=$i verdict=$v boot_rc=$brc graded=[${g% }] $wall trapped=${trapped:-?} ledger=[${led% }]" >> "$OUT"
            [ "$r" = cup8bench ] && grep -a '^GUEST_BENCH_INIT_MS=\|^GUEST_BENCH_CTX_MS=\|^GUEST_BENCH_MODULE_MS=\|^GUEST_BSUM ' "$probe" \
                | sed "s/^/CL_BENCH mode=guest rep=$i /" >> "$OUT"
            tail -1 "$OUT"
        done
    done
    ;;
*) sed -n '2,32p' "$0"; exit 2 ;;
esac
echo "CUDA_LADDER_DONE=$(date -Is)" >> "$OUT"
cat "$OUT"
