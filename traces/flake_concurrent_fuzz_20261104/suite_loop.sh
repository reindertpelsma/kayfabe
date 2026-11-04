#!/usr/bin/env bash
# Flake hunt 2026-11-04 (widened): run the WHOLE thin fast suite (30 arms, suite order, one
# boot per arm) R times; per run keep the scoreboard, every arm's REFUSED walk / CUDA_ERROR lines,
# the concurrent-fuzz seed and FUZZ_REASON, the host Xid lines with timestamps, and the full logs
# of every non-PASS arm (or arm with a REFUSED walk line).
# usage: suite_loop.sh <phase-tag> <runs>    — ON THE BOX, from the checkout. Output /root/cfz/<phase>/
set -uo pipefail
PH=${1:?phase}; R=${2:?runs}
REPO=$(cd "$(dirname "$0")/../.." && pwd)
BENCH=${BENCH_DIR:-/workspace/bench}
OUT=/root/cfz/$PH; mkdir -p "$OUT/fail"
echo "SUITE_LOOP_START phase=$PH runs=$R rev=$(git -C "$REPO" rev-parse --short=9 HEAD) at=$(date -Is)" >> "$OUT/run.log"
for r in $(seq 1 "$R"); do
    tag="sl${PH}_$r"
    x0=$(dmesg | grep -c 'NVRM: Xid')
    dmesg_mark=$(dmesg | wc -l)
    KF_DEVICE=kf3 bash "$REPO/scripts/fastguest/fast_suite.sh" "$tag" 180 > "$OUT/suite_$r.out" 2>&1
    src=$?
    x1=$(dmesg | grep -c 'NVRM: Xid')
    dmesg | tail -n +"$((dmesg_mark + 1))" | grep -a 'NVRM' | head -40 > "$OUT/suite_${r}_host_nvrm.txt"
    : > "$OUT/suite_${r}_refused.txt"
    for q in "$BENCH"/fast_${tag}_*_qemu.log; do
        [ -f "$q" ] || continue
        a=$(basename "$q"); a=${a#fast_${tag}_}; a=${a%_qemu.log}
        n=$(grep -ac 'REFUSED walk\|CUDA_ERROR' "$q")
        if [ "$n" -gt 0 ]; then
            echo "arm=$a n=$n" >> "$OUT/suite_${r}_refused.txt"
            grep -a 'REFUSED walk\|CUDA_ERROR' "$q" | head -5 | cut -c1-300 | sed 's/^/    /' >> "$OUT/suite_${r}_refused.txt"
        fi
        v=$(grep -a "FAST_CELL_ARM arm=$a " "$OUT/suite_$r.out" | grep -o 'verdict=[A-Z_]*' | cut -d= -f2)
        if [ "${v:-?}" != PASS ] || [ "$n" -gt 0 ]; then
            for f in "$BENCH"/fast_${tag}_${a}_*; do cp "$f" "$OUT/fail/r${r}_$(basename "$f")"; done
        fi
    done
    fz="$BENCH/fast_${tag}_concurrent-fuzz_serial.log"
    seed=$(grep -ao -- '--seed [0-9]*' "$fz" 2>/dev/null | head -1 | awk '{print $2}')
    reason=$(grep -ao 'FUZZ_REASON=[A-Z_/]*' "$fz" 2>/dev/null | tail -1 | cut -d= -f2)
    grep -a -E 'FUZZ_|W385 control|RUNG_concurrent_fuzz' "$fz" 2>/dev/null | tr -d '\r' | cut -c1-300 > "$OUT/suite_${r}_fuzz.txt"
    echo "SUITE_RUN phase=$PH r=$r rc=$src $(grep -a 'FAST_SUITE_PASS=' "$OUT/suite_$r.out" | tail -1) xid_delta=$((x1-x0)) fuzz_seed=${seed:-?} fuzz_reason=${reason:-?} refused_arms=$(grep -c '^arm=' "$OUT/suite_${r}_refused.txt")" >> "$OUT/run.log"
    rm -f "$BENCH"/fast_${tag}_* 2>/dev/null
done
echo "SUITE_LOOP_EXIT phase=$PH at=$(date -Is)" >> "$OUT/run.log"
