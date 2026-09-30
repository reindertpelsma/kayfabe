#!/usr/bin/env bash
# ★ dbfast_lane.sh — the doorbell fast path's ON/OFF evidence on ONE provisioned box, strictly
# serial (docs/design/V3_DOORBELL_IOEVENTFD.md §7). Run it FROM the checkout whose kf3 binary
# `build_kf3.sh` built (kf3-bins/<this rev>/), e.g. merge_check.sh's verify worktree, after the
# merge bar (which is the fast path OFF: the property's default).
#
#   dbfast_lane.sh <tag> [steps=suite_on,exit,ladder,gpufree]
#     suite_on  the 30-arm thin suite with doorbell-ioeventfd=on (budget 180): FAST_SUITE_PASS=…
#               plus the fast path's own counters from every arm's QEMU log
#     exit      the vCPU cost of one doorbell store, in the guest (dbfast_exit_hook.sh): one fat-guest
#               boot with the fast path ON (+ the measurement probe), one OFF; both with dummy-bar
#     ladder    the CUDA ladder guest arm (cup2, cup3, cup8, cup8bench), OFF then ON
#     gpufree   the real-KVM tests and the GPU-free benchmark (dbfast_kvm, dbfast_exhaust, dbfast_bench)
#     launch    cup8bench alone, DBL_REPS (3) boots each: OFF, ON, and ON + KF3_DBFAST_SPIN_US=DBL_SPIN_US
#               (100) — the launch-latency trade (single synchronous vs batched launches) with variance
# ⚠ Everything on a vast box is NESTED. One DBL_<step>_RC line per step; the log ends with EXIT.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
T=${1:?tag}; STEPS=${2:-suite_on,exit,ladder,gpufree}
BENCH=${BENCH_DIR:-/workspace/bench}
OUT=$BENCH/dbfast; mkdir -p "$OUT"
LOG=$OUT/$T.log
[ ! -e "$LOG" ] || { echo "refusing to overwrite $LOG" >&2; exit 2; }
exec >"$LOG" 2>&1
trap 'echo "EXIT rc=$? $(date -Is)"' EXIT
REV=$(git -C "$REPO" rev-parse --short=8 HEAD)
echo "START $(date -Is) tag=$T steps=$STEPS rev=$REV nproc=$(nproc) kernel=$(uname -r) (NESTED: a vast KVM guest)"
export KF_DEVICE=kf3 PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$BENCH/kf-verify-target}
export CLIENT=$CARGO_TARGET_DIR/release/kayfabe-rm-ladder KF_LADDER=$CARGO_TARGET_DIR/release/kayfabe-rm-ladder
[ -x "$BENCH/kf3-bins/$REV/qemu-system-x86_64" ] || { echo "no kf3 binary for $REV — run build_kf3.sh from this checkout"; exit 2; }
ON=doorbell-ioeventfd=on
has() { case ",$STEPS," in *",$1,"*) return 0 ;; esac; return 1; }
gpu_idle() {  # the previous VM can hold its reservation for a moment after QEMU exits
    for _ in $(seq 1 60); do
        u=$(nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits 2>/dev/null | head -1 | tr -dc 0-9)
        [ -n "$u" ] && [ "$u" -le 512 ] && return 0; sleep 0.5
    done
}

if has suite_on; then
    KF3_DEV_EXTRA=$ON bash "$REPO/scripts/fastguest/fast_suite.sh" "${T}_on" 180 > "$OUT/${T}_suite_on.run" 2>&1
    echo "DBL_SUITE_ON_RC=$?"
    grep -a '^FAST_SUITE_PASS' "$BENCH/${T}_on_suite.out"
    # The fast path's own view of every arm (the device prints its status line at exit).
    for q in "$BENCH"/fast_"${T}"_on_*_qemu.log; do
        [ -e "$q" ] || continue
        a=$(basename "$q" _qemu.log); a=${a#fast_"${T}"_on_}
        s=$(grep -ao 'dbfast\[[^]]*\]' "$q" | tail -1 | cut -c1-240)
        l=$(grep -ao 'DOORBELL-LEDGER tok=[^ ]* .*' "$q" | sed -n 's/.* trap=\([0-9]*\) fast=\([0-9]*\).*/t\1f\2/p' | tr '\n' ' ')
        echo "DBL_ARM $a ${s:-no-status} pt_ledger=[${l% }]"
    done
fi

if has exit; then
    for mode in on off; do
        gpu_idle
        extra=dummy-bar=on; [ "$mode" = on ] && extra=$ON,dummy-bar=on
        KF3_DBFAST_PROBE=0x007f07ff KF3_DEV_EXTRA=$extra POST_CAPTURE_HOOK="$HERE/dbfast_exit_hook.sh" \
            GQ_TIMEOUT=900 bash "$HERE/boot_capture.sh" "${T}_exit_$mode" > "$OUT/${T}_exit_$mode.driver" 2>&1
        echo "DBL_EXIT_${mode}_RC=$?"
        grep -ah '^DBX_' "$BENCH/run_${T}_exit_${mode}_probe.log" 2>/dev/null | sed "s/^/DBL_EXIT mode=$mode /"
        grep -ao 'MEASUREMENT PROBE.*\|dbfast\[[^]]*\]' "$BENCH/run_${T}_exit_${mode}_qemu.log" 2>/dev/null | tail -2 | cut -c1-300 | sed "s/^/DBL_EXIT mode=$mode /"
    done
fi

if has ladder; then
    for mode in off on; do
        gpu_idle
        extra=""; [ "$mode" = on ] && extra=$ON
        KF3_DEV_EXTRA=$extra bash "$HERE/cuda_ladder.sh" guest "${T}_$mode" 1 > "$OUT/${T}_ladder_$mode.run" 2>&1
        echo "DBL_LADDER_${mode}_RC=$?"
        grep -a '^CL_ROW\|^CL_BENCH' "$BENCH/cl_${T}_${mode}_guest.out" | cut -c1-400 | sed "s/^/DBL_LADDER mode=$mode /"
        for q in "$BENCH"/run_cl_"${T}"_"${mode}"_*_qemu.log; do
            [ -e "$q" ] || continue
            echo "DBL_LADDER mode=$mode $(basename "$q") $(grep -ao 'dbfast\[[^]]*\]' "$q" | tail -1 | cut -c1-260)"
            grep -ao 'DOORBELL-LEDGER tok=[^ ]* route=[a-z]* .*' "$q" | cut -c1-260 | sed "s/^/DBL_LEDGER mode=$mode /"
        done
    done
fi

if has launch; then
    for mode in off on spin; do
        gpu_idle
        extra=""; [ "$mode" != off ] && extra=$ON
        if [ "$mode" = spin ]; then export KF3_DBFAST_SPIN_US=${DBL_SPIN_US:-100}; else unset KF3_DBFAST_SPIN_US; fi
        KF3_DEV_EXTRA=$extra bash "$HERE/cuda_ladder.sh" guest "${T}_l$mode" "${DBL_REPS:-3}" cup8bench \
            > "$OUT/${T}_launch_$mode.run" 2>&1
        echo "DBL_LAUNCH_${mode}_RC=$? spin=${KF3_DBFAST_SPIN_US:-0}"
        grep -a '^CL_ROW\|GUEST_BSUM' "$BENCH/cl_${T}_l${mode}_guest.out" | cut -c1-330 | sed "s/^/DBL_LAUNCH mode=$mode /"
        for q in "$BENCH"/run_cl_"${T}"_l"${mode}"_*_qemu.log; do
            [ -e "$q" ] || continue
            echo "DBL_LAUNCH mode=$mode $(basename "$q") $(grep -ao 'dbfast\[[^]]*\]' "$q" | tail -1 | cut -c1-300)"
        done
    done
    unset KF3_DBFAST_SPIN_US
fi

if has gpufree; then
    ( cd "$REPO" && cargo test -q -p kf-chan --test dbfast_kvm --test dbfast_exhaust -- --nocapture ) \
        > "$OUT/${T}_gpufree_tests.log" 2>&1
    echo "DBL_GPUFREE_TESTS_RC=$?"
    grep -a 'test result\|CHURN\|EXHAUST\|KVM-GATE' "$OUT/${T}_gpufree_tests.log"
    ( cd "$REPO" && KF_DBFAST_BENCH=1 cargo test -q --release -p kf-chan --test dbfast_bench -- --nocapture --test-threads 1 ) \
        > "$OUT/${T}_gpufree_bench.log" 2>&1
    echo "DBL_GPUFREE_BENCH_RC=$?"
    grep -a 'DBFAST-BENCH\|^  \|test result' "$OUT/${T}_gpufree_bench.log"
fi
