#!/usr/bin/env bash
# ★★★★★ THE CUDA LADDER ON kf3 — cup2 → cup3 → cup8 → cup8bench, guest arm and host arm.
#
# > Owner, 2026-09-25: *"Test the CUDA ladder. How much slower is the raw client compared to
# > host now."*
#
# ## Why the THIN guest, not the fat qcow2 guest
#
# The ladder's hooks (`scripts/bench/cup*_hook.sh`) were written for the old tree's fat guest:
# boot a 6 GiB qcow2, wait for sshd, `gcc` INSIDE the guest, run over ssh. On kf3 nothing boots
# that path, and every one of its known traps (ssh races, dmesg not in the serial log, gcc in
# the guest compiling against a different header than the host arm) is a reason not to revive
# it for a measurement. The thin guest (`build_fast_guest.sh`, host mode) already boots the
# host's own `nvidia*.ko` against kf3 30/30; what it lacked was only the CUDA userspace. So:
#
#   1. `build`  — compile the ladder ONCE, on the host, against the host libcuda (the box has no
#                 toolkit: `scripts/bench/cuda_min/cuda.h` declares the dozen entry points).
#   2. `initrd` — `build_fast_guest.sh` with `KF_CUDA_BINS` carries those SAME binaries plus
#                 libcuda + the PTX JIT (version-checked against the guest's nvidia.ko).
#   3. `guest`  — one boot per rung per rep (`KF_CUDA=<rung>` → `/init` runs it instead of the
#                 raw client), so the device's per-token DOORBELL-LEDGER belongs to that run.
#   4. `host`   — the same binaries on bare metal, same box, same env.
#
# ⇒ One source, one compiler, one binary, two platforms: the ratio compares platforms.
#
# ## Grading — the workloads' own un-forgeable values, never a return code alone
#   cup2  `^CUP2_VAL=0xabcd1234`   (a CE round-trip)
#   cup3  `^CUP3_VAL=43`           (14*3+1 computed by a shader — nothing in the stack forges it)
#   cup8  `^CUP8_BAD=0` `^CUP8_MAXERR=0`   (2048² fp32 matmul, exact closed form)
#   cup8bench `BENCH_VERDICT: PASS` + `B<N>_BAD=0` (every timed iteration verified, C poisoned)
# plus, guest arm only, run_fast_guest.sh's per-token ledger gate (`forwarded>0` per rung token).
#
# usage: cuda_ladder.sh build
#        cuda_ladder.sh initrd
#        cuda_ladder.sh guest <tag> [reps=3] [rungs=cup2,cup3,cup8,cup8bench]
#        cuda_ladder.sh host  <tag> [reps=3] [rungs=cup2,cup3,cup8,cup8bench]
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
BENCH=${BENCH_DIR:-/workspace/bench}
BINS=${KF_CUDA_BINS:-$BENCH/cudaladder-bins}
# cup8bench: N=16 isolates per-launch overhead; N=2048 is the cup8 matmul. 20 timed iters each.
export BENCH_SIZES=${BENCH_SIZES:-16,2048} BENCH_ITERS=${BENCH_ITERS:-20} BENCH_BATCH=${BENCH_BATCH:-10}
die() { echo "cuda_ladder: $*" >&2; exit 2; }

grade() {  # $1 = rung, $2 = output file → prints graded lines, returns 0 iff graded PASS
    local r=$1 f=$2
    case "$r" in
        cup2) grep -a '^CUP2_VAL=' "$f" | tail -1; grep -aq '^CUP2_VAL=0xabcd1234' "$f" ;;
        cup3) grep -a '^CUP3_VAL=' "$f" | tail -1; grep -aq '^CUP3_VAL=43$' "$f" ;;
        cup8) grep -a '^CUP8_BAD=\|^CUP8_MAXERR=' "$f" | tail -2
              grep -aq '^CUP8_BAD=0$' "$f" && grep -aq '^CUP8_MAXERR=0$' "$f" ;;
        cup8bench) grep -a '^BENCH_VERDICT:\|^B[0-9]*_BAD=' "$f"
              grep -aq '^BENCH_VERDICT: PASS' "$f" && ! grep -aq '^B[0-9]*_BAD=[1-9]' "$f" ;;
    esac
}

case "${1:-}" in
build)
    command -v gcc >/dev/null || die "no gcc on the host"
    [ -e /usr/lib/x86_64-linux-gnu/libcuda.so ] || [ -e /usr/lib/x86_64-linux-gnu/libcuda.so.1 ] \
        || die "no libcuda on the host"
    rm -rf "$BINS"; mkdir -p "$BINS"
    # ⚠ -lcuda needs the unversioned link name; link against the .so.1 by path if it is absent.
    L=-lcuda; [ -e /usr/lib/x86_64-linux-gnu/libcuda.so ] || L=/usr/lib/x86_64-linux-gnu/libcuda.so.1
    I="-I$REPO/scripts/bench/cuda_min"
    gcc -O0 $I -o "$BINS/cup2" "$REPO/archive/nvkvm/tests/mode2/cup2.c" $L       || die "cup2 build failed"
    gcc -O0 $I -o "$BINS/cup3" "$REPO/scripts/bench/cup3.c" $L                   || die "cup3 build failed"
    gcc -O0 $I -o "$BINS/cup8" "$REPO/scripts/bench/cup8.c" $L -lm               || die "cup8 build failed"
    gcc -O2    -o "$BINS/cup8bench" "$REPO/scripts/bench/cup8bench.c" $L -lm     || die "cup8bench build failed"
    for s in archive/nvkvm/tests/mode2/cup2.c scripts/bench/cup3.c scripts/bench/cup8.c scripts/bench/cup8bench.c; do
        printf '   %-40s md5 %s\n' "$s" "$(md5sum < "$REPO/$s" | cut -d' ' -f1)"
    done
    echo "CUDA_LADDER_BUILT $BINS rev=$(git -C "$REPO" rev-parse --short=8 HEAD) libcuda=$(readlink -f /usr/lib/x86_64-linux-gnu/libcuda.so.1)"
    ;;
initrd)
    [ -x "$BINS/cup3" ] || die "no ladder binaries in $BINS — run: $0 build"
    KF_FROM_HOST=1 KF_CUDA_BINS="$BINS" bash "$HERE/build_fast_guest.sh" none "$BENCH/fastguest"
    ;;
guest|host)
    MODE=$1; TAG=${2:?tag}; REPS=${3:-3}; RUNGS=${4:-cup2,cup3,cup8,cup8bench}
    [ -x "$BINS/cup3" ] || die "no ladder binaries in $BINS — run: $0 build"
    OUT=$BENCH/cl_${TAG}_${MODE}.out
    { echo "CUDA_LADDER_STARTED=$(date -Is) mode=$MODE reps=$REPS rungs=$RUNGS"
      echo "rev=$(git -C "$REPO" rev-parse --short=8 HEAD) env: BENCH_SIZES=$BENCH_SIZES BENCH_ITERS=$BENCH_ITERS BENCH_BATCH=$BENCH_BATCH"
    } > "$OUT"
    for r in $(echo "$RUNGS" | tr ',' ' '); do
        for i in $(seq 1 "$REPS"); do
            log=$BENCH/cl_${TAG}_${MODE}_${r}_$i.log
            if [ "$MODE" = host ]; then
                exec 9>"${KF_LOCK:-/tmp/kayfabe-fastguest.lock}"; flock 9
                t0=$(date +%s%N)
                ( cd /tmp && timeout 600 "$BINS/$r" ) > "$log" 2>&1; prc=$?
                t1=$(date +%s%N)
                exec 9>&-
                wall=$(( (t1 - t0) / 1000000 ))
                # the same anchored keys /init derives in the guest
                case "$r" in
                    cup2) echo "CUP2_VAL=$(sed -n 's/^CE rv=\(0x[0-9a-f]*\) .*/\1/p' "$log" | tail -1)" >> "$log" ;;
                    cup3) echo "CUP3_VAL=$(sed -n 's/^KERNEL rv=\([0-9]*\) .*/\1/p' "$log" | tail -1)" >> "$log" ;;
                    cup8) l=$(grep '^CUP8 RESULT ' "$log" | tail -1)
                          echo "CUP8_BAD=$(echo "$l" | sed -n 's/.* bad=\([0-9]*\) .*/\1/p')" >> "$log"
                          echo "CUP8_MAXERR=$(echo "$l" | sed -n 's/.* maxerr=\([^ ]*\) .*/\1/p')" >> "$log" ;;
                esac
                fwd="n/a"; boot="n/a"
            else
                # ★ env for cup8bench reaches /init through the kernel command line (no whitespace).
                KF_DEVICE=kf3 KF_CUDA=$r NVKVM_RAM_MB=${NVKVM_RAM_MB:-4096} \
                KF_APPEND="BENCH_SIZES=$BENCH_SIZES BENCH_ITERS=$BENCH_ITERS BENCH_BATCH=$BENCH_BATCH ${KF_APPEND:-}" \
                    bash "$HERE/run_fast_guest.sh" "cl_${TAG}_${r}_$i" "${KF_CUDA_BUDGET:-240}" > "$log.run" 2>&1
                ser=$BENCH/fast_cl_${TAG}_${r}_${i}_serial.log
                cp "$ser" "$log" 2>/dev/null || : > "$log"
                cat "$log.run" >> "$log"
                prc=$(grep -ao "FASTGUEST: CUDA $r rc=[0-9]*" "$ser" | tail -1 | sed 's/.*rc=//')
                wall=$(grep -ao "FASTGUEST: CUDA $r rc=[0-9]* wall_ms=[0-9]*" "$ser" | tail -1 | sed 's/.*wall_ms=//')
                fwd=$(grep -ao 'DOORBELL-LEDGER tok=[^ ]* .*' "$BENCH/fast_cl_${TAG}_${r}_${i}_qemu.log" 2>/dev/null \
                      | sed -n 's/.*tok=\(0x[0-9a-f]*\) .*emulated=\([0-9]*\) forwarded=\([0-9]*\).*/\1:e\2f\3/p' | sort -u | tr '\n' ' ')
                boot=$(grep -ao 'FAST_VERDICT=[A-Z]*' "$log.run" | tail -1)
            fi
            g=$(grade "$r" "$log" | tr '\n' ' '); grade "$r" "$log" >/dev/null && v=PASS || v=FAIL
            echo "CL_ROW mode=$MODE rung=$r rep=$i verdict=$v rc=${prc:-?} wall_ms=${wall:-?} graded=[${g% }] ledger=[${fwd% }] ${boot}" >> "$OUT"
            if [ "$r" = cup8bench ]; then
                grep -a '^BENCH_INIT_MS=\|^BENCH_CTX_MS=\|^BENCH_MODULE_MS=\|^BSUM ' "$log" \
                    | sed "s/^/CL_BENCH mode=$MODE rep=$i /" >> "$OUT"
            fi
            tail -1 "$OUT"
        done
    done
    echo "CUDA_LADDER_DONE=$(date -Is)" >> "$OUT"
    cat "$OUT"
    ;;
*) sed -n '2,40p' "$0"; exit 2 ;;
esac
