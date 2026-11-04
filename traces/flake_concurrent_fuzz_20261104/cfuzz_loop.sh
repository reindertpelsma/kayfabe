#!/usr/bin/env bash
# Flake hunt 2026-11-04: run `--concurrent-fuzz` in the thin fast guest N times, one fresh QEMU
# per iteration (fast_suite.sh, exactly as the suite runs it), and record per iteration the
# verdict, secs, client_ms, client rc, the client's FUZZ_* summary, the host Xid delta, and on
# failure the full serial/QEMU(kf3) logs.
#
# usage: cfuzz_loop.sh <phase-tag> <n> [pre-arm]
#   pre-arm: an arm run (own boot) before each concurrent-fuzz iteration, e.g. --map-stress,
#            to reproduce the suite's order (map-stress is the arm just before it).
# Run ON THE BOX from the kayfabe checkout. Output: /root/cfz/<phase>/
set -uo pipefail
PH=${1:?phase}; N=${2:?n}; PRE=${3:-}
REPO=$(cd "$(dirname "$0")/../.." && pwd)
BENCH=${BENCH_DIR:-/workspace/bench}
OUT=/root/cfz/$PH; mkdir -p "$OUT/fail"
SUM=$OUT/summary.tsv
rev=$(git -C "$REPO" rev-parse --short=9 HEAD)
echo "CFZ_START phase=$PH n=$N pre=${PRE:-none} rev=$rev at=$(date -Is)" >> "$OUT/run.log"
[ -f "$SUM" ] || printf 'iter\tverdict\tsecs\tclient_ms\tclient_rc\tfuzz_reason\tseed\txid_delta\txid_codes\tpre_verdict\tarms\n' > "$SUM"
xidcount() { dmesg 2>/dev/null | grep -c 'NVRM: Xid' ; }
for i in $(seq 1 "$N"); do
    pre_v=-
    x0=$(xidcount)
    if [ -n "$PRE" ]; then
        KF_DEVICE=kf3 bash "$REPO/scripts/fastguest/fast_suite.sh" "cfz${PH}_pre$i" 180 "$PRE" > "$OUT/pre_$i.out" 2>&1
        pre_v=$(grep -o 'FAST_CELL_ARM .*verdict=[A-Z_]*' "$OUT/pre_$i.out" | grep -o 'verdict=[A-Z_]*' | cut -d= -f2)
    fi
    x1=$(xidcount)
    tag="cfz${PH}_$i"
    KF_DEVICE=kf3 bash "$REPO/scripts/fastguest/fast_suite.sh" "$tag" 180 --concurrent-fuzz > "$OUT/it_$i.out" 2>&1
    x2=$(xidcount)
    ser="$BENCH/fast_${tag}_concurrent-fuzz_serial.log"
    qlog="$BENCH/fast_${tag}_concurrent-fuzz_qemu.log"
    cell=$(grep -a 'FAST_CELL_ARM' "$OUT/it_$i.out" | tail -1)
    v=$(echo "$cell" | grep -o 'verdict=[A-Z_]*' | cut -d= -f2)
    s=$(echo "$cell" | grep -o 'secs=[0-9?-]*' | cut -d= -f2)
    c=$(echo "$cell" | grep -o 'client_ms=[0-9?]*' | cut -d= -f2)
    crc=$(grep -ao 'FASTGUEST: client rc=[0-9]*' "$ser" 2>/dev/null | tail -1 | grep -o '[0-9]*$')
    reason=$(grep -ao 'FUZZ_REASON=[A-Z_/]*' "$ser" 2>/dev/null | tail -1 | cut -d= -f2)
    seed=$(grep -ao -- '--seed [0-9]*' "$ser" 2>/dev/null | head -1 | awk '{print $2}')
    arms=$(grep -ao 'FUZZ_ARM=[a-z-]* verdict=[A-Z]* why=[A-Z_]*' "$ser" 2>/dev/null | sed 's/FUZZ_ARM=//; s/ verdict=/:/; s/ why=/:/' | tr '\n' ' ')
    codes=$(dmesg 2>/dev/null | grep 'NVRM: Xid' | tail -n $(( x2 - x0 > 0 ? x2 - x0 : 0 )) | grep -o 'Xid ([^)]*): [0-9]*' | awk '{print $NF}' | tr '\n' ',' )
    [ $(( x2 - x0 )) -gt 0 ] || codes=-
    # the fuzz window's own Xids (x2-x1) vs the pre-arm window's (x1-x0)
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$i" "${v:-?}" "${s:-?}" "${c:-?}" "${crc:-?}" \
        "${reason:-?}" "${seed:-?}" "pre=$((x1-x0)),fuzz=$((x2-x1))" "$codes" "$pre_v" "$arms" >> "$SUM"
    # per-iteration client summary lines, always (small)
    grep -a -E 'FUZZ_|W385 (CONTROL|control|ARM|FUZZ CLEAN|replay)|RUNG_concurrent_fuzz|RUNGCTL|REV_UNDER_TEST|UNMEASURED|FASTGUEST: (ready|client rc|DONE)' "$ser" 2>/dev/null \
        | tr -d '\r' | cut -c1-400 > "$OUT/it_${i}_fuzz.txt"
    if [ "${v:-?}" != PASS ]; then
        cp "$ser" "$OUT/fail/it_${i}_serial.log" 2>/dev/null
        cp "$qlog" "$OUT/fail/it_${i}_qemu.log" 2>/dev/null
        cp "$BENCH/fast_${tag}_concurrent-fuzz_ttyS0.log" "$OUT/fail/it_${i}_ttyS0.log" 2>/dev/null
        dmesg 2>/dev/null | tail -60 > "$OUT/fail/it_${i}_dmesg_tail.txt"
        [ -n "$PRE" ] && cp "$BENCH/fast_cfz${PH}_pre${i}_${PRE#--}_serial.log" "$OUT/fail/it_${i}_pre_serial.log" 2>/dev/null
    fi
    # keep the box's bench dir from filling: drop this iteration's big logs once copied
    rm -f "$BENCH"/fast_${tag}_* "$BENCH"/fast_cfz${PH}_pre${i}_* 2>/dev/null
    echo "CFZ_ITER phase=$PH i=$i verdict=${v:-?} secs=${s:-?} client_ms=${c:-?} rc=${crc:-?} reason=${reason:-?}" >> "$OUT/run.log"
done
echo "CFZ_EXIT phase=$PH pass=$(awk -F'\t' 'NR>1&&$2=="PASS"' "$SUM" | wc -l) total=$(( $(wc -l < "$SUM") - 1 )) at=$(date -Is)" >> "$OUT/run.log"
