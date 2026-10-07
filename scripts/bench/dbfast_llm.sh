#!/usr/bin/env bash
# ★ dbfast_llm.sh — LLM decode with the doorbell fast path ON vs OFF, paired and alternated, on ONE
# provisioned box (docs/design/V3_DOORBELL_IOEVENTFD.md §7). ⚠ On a vast box: NESTED.
#
#   dbfast_llm.sh <tag> <kf3-binary> [order=prov,on,off,spin,on,off]
#     prov  llm_parity_box.sh gprov,hprov,host (guest venv + model, host venv + model, host lane)
#     on    guest_pm lane with -device …,doorbell-ioeventfd=on
#     off   guest_pm lane with the property's default (off)
#     spin  guest_pm lane ON + KF3_DBFAST_SPIN_US=${DBL_SPIN_US:-100} (the spin-then-park experiment)
# Each lane boot is `llm_parity_box.sh <tag>_<step><n>` (outputs in /workspace/bench/llm/); the
# identified-QEMU counters (LLM_KVM_EXITS, LLM_DOORBELLS, LLM_THREAD_CPU) ride along. Smaller
# matrix than the parity default so a pair fits an afternoon: LP_SHORT_PROCS=1, LP_LONG_PROCS=2.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
T=${1:?tag}; QB=${2:?kf3 binary}; ORDER=${3:-prov,on,off,spin,on,off}
OUT=/workspace/bench/llm; mkdir -p "$OUT"
LOG=$OUT/${T}_dbfast_llm.log
exec >>"$LOG" 2>&1
trap 'echo "EXIT rc=$? $(date -Is)"' EXIT
echo "START $(date -Is) tag=$T qb=$QB order=$ORDER rev=$(git -C "$HERE" rev-parse --short=8 HEAD) (NESTED)"
export LP_SHORT_PROCS=${LP_SHORT_PROCS:-1} LP_LONG_PROCS=${LP_LONG_PROCS:-2} LP_LONG=${LP_LONG:-512,2048}
n=0
for step in $(echo "$ORDER" | tr ',' ' '); do
    n=$((n + 1))
    case "$step" in
        prov) bash "$HERE/llm_parity_box.sh" "${T}_prov" "$QB" gprov,hprov,host ;;
        on)   KF3_DEV_EXTRA=doorbell-ioeventfd=on bash "$HERE/llm_parity_box.sh" "${T}_on$n" "$QB" guest_pm ;;
        off)  bash "$HERE/llm_parity_box.sh" "${T}_off$n" "$QB" guest_pm ;;
        spin) KF3_DEV_EXTRA=doorbell-ioeventfd=on KF3_DBFAST_SPIN_US=${DBL_SPIN_US:-100} \
                  bash "$HERE/llm_parity_box.sh" "${T}_spin$n" "$QB" guest_pm ;;
        *) echo "unknown step $step"; continue ;;
    esac
    echo "DBLLM_STEP $n $step rc=$? $(date -Is)"
done
