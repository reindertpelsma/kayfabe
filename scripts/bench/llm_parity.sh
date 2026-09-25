#!/usr/bin/env bash
# ★★★★★ LLM PARITY MATRIX — one script, three lanes, the SAME runner (`run_llm.py`) on each.
#
#   llm_parity.sh local <lane>   # host / bare-metal lane: runs run_llm.py directly
#                                #   env LLM_PY (python), LLM_ROOT (dir holding hf/; run_llm.py is
#                                #   copied there from this tree first)
#   llm_parity.sh hook  [tag]    # POST_CAPTURE_HOOK for boot_capture.sh (fat guest, KF_DEVICE=kf3):
#                                #   installs THIS tree's run_llm.py into /opt/llm first, then runs
#                                #   the same matrix inside the guest over gssh_nv
#
# The matrix (every row a SEPARATE process, so each carries its own cold start):
#   short : LP_SHORT_PROCS (3) x the recorded measurement, unchanged — 16 tokens greedy, default
#           mode (no timeline), LLM_MS = cold generate() wall, the basis HOST_LLM_TOK_PER_S uses.
#   graph : (diagnostic, LP_GRAPH=512,2048 x LP_GRAPH_PROCS) run_llm_graph.py — the decode step as
#           ONE replayed CUDA graph, so per-launch (doorbell) cost is ~removed; separates it from the rest.
#   long  : for each ntok in LP_LONG (512,2048): LP_LONG_PROCS (3) processes with
#           LLM_TIMELINE=1 LLM_MIN_NTOK=1 LLM_REPS=LP_WARM (1): per-stage init times, then one cold
#           and LP_WARM warm generate() runs, each with ttft and steady-state decode tok/s.
# Every process is also timed from OUTSIDE (LP ... ext_ms=), which includes interpreter start and
# teardown — the number a user waits for.
#
# Output: one `LP lane=<l> kind=<k> ntok=<n> proc=<i> <LLM line>` per runner fact, plus
# `LP_EXT lane=… ext_ms=… rc=…` per process. `llm_parity_summary.py` turns the files into tables.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MODE=${1:-}; LANE=${2:-}
SHORT_PROCS=${LP_SHORT_PROCS:-3}
LONG=${LP_LONG-512,2048}
LONG_PROCS=${LP_LONG_PROCS:-3}
WARM=${LP_WARM:-1}
GRAPH=${LP_GRAPH-512,2048}   # diagnostic arm (run_llm_graph.py): CUDA-graph decode; empty = off
GRAPH_PROCS=${LP_GRAPH_PROCS:-3}
TMO=${LLM_TIMEOUT:-3600}
MODEL=${LLM_MODEL:-Qwen/Qwen2-0.5B-Instruct}

case "$MODE" in
local)
    [ -n "$LANE" ] || { sed -n 2,20p "$0"; exit 2; }
    ROOT=${LLM_ROOT:?LLM_ROOT}; PY=${LLM_PY:?LLM_PY}
    cp "$HERE/run_llm.py" "$HERE/run_llm_graph.py" "$ROOT/" || exit 2
    run() { local r=${RUNNER:-run_llm.py}
            ( cd "$ROOT" && env HF_HOME="$ROOT/hf" LLM_DEVICE=cuda LLM_MODEL="$MODEL" "$@" \
              timeout "$TMO" "$PY" "$r" 2>&1 ); }
    SNAP=$(ls "$ROOT/hf/hub/models--${MODEL//\//--}/snapshots" 2>/dev/null | tr '\n' ' ')
    ;;
hook)
    LANE=guest
    G="$HERE/gssh_nv"
    $G true >/dev/null 2>&1 || { echo "LP_OUTCOME=(E) UNMEASURED_GUEST_UNREACHABLE"; exit 0; }
    $G 'cat > /opt/llm/run_llm.py' < "$HERE/run_llm.py" || { echo "LP_OUTCOME=runner-install-failed"; exit 0; }
    $G 'cat > /opt/llm/run_llm_graph.py' < "$HERE/run_llm_graph.py" || { echo "LP_OUTCOME=runner-install-failed"; exit 0; }
    run() { $G "cd /opt/llm && env HF_HOME=/opt/llm/hf LLM_DEVICE=cuda LLM_MODEL=$MODEL $* \
                timeout $TMO /home/ubuntu/llmvenv/bin/python ${RUNNER:-run_llm.py} 2>&1" 2>&1 | tr -d '\r'; }
    SNAP=$($G "ls /opt/llm/hf/hub/models--${MODEL//\//--}/snapshots" 2>/dev/null | tr -d '\r' | tr '\n' ' ')
    echo "LP_GUEST_SMI=$($G 'nvidia-smi --query-gpu=name,driver_version --format=csv,noheader' 2>&1 | tr -d '\r' | head -1)"
    echo "LP_GUEST_NPROC=$($G nproc 2>&1 | tr -d '\r') LP_GUEST_MEM=$($G "free -m | awk '/Mem:/{print \$2}'" 2>&1 | tr -d '\r')"
    ;;
*) sed -n 2,20p "$0"; exit 2 ;;
esac

echo "LP_START lane=$LANE date=$(date -Is) model=$MODEL snapshot=[${SNAP% }] short=$SHORT_PROCS long=$LONG x$LONG_PROCS warm=$WARM"
one() {  # $1 kind, $2 ntok, $3 proc, rest: env
    local k=$1 n=$2 i=$3; shift 3
    local t0 t1 out rc
    t0=$(date +%s%N)
    out=$(run LLM_NTOK="$n" "$@"); rc=$?
    t1=$(date +%s%N)
    echo "$out" | grep -a '^LLM_\|^TORCH_' | sed "s/^/LP lane=$LANE kind=$k ntok=$n proc=$i /"
    echo "LP_EXT lane=$LANE kind=$k ntok=$n proc=$i ext_ms=$(( (t1 - t0) / 1000000 )) rc=$rc"
    grep -q '^LLM_OK=1' <<<"$out" || echo "$out" | tail -20 | sed "s/^/LP_FAILTAIL lane=$LANE kind=$k ntok=$n proc=$i /"
}
for i in $(seq 1 "$SHORT_PROCS"); do one short 16 "$i"; done
for n in $(echo "$LONG" | tr ',' ' '); do
    for i in $(seq 1 "$LONG_PROCS"); do one long "$n" "$i" LLM_TIMELINE=1 LLM_MIN_NTOK=1 LLM_REPS="$WARM"; done
done
for n in $(echo "$GRAPH" | tr ',' ' '); do
    for i in $(seq 1 "$GRAPH_PROCS"); do RUNNER=run_llm_graph.py one graph "$n" "$i" LLM_REPS="$WARM"; done
done
echo "LP_DONE lane=$LANE date=$(date -Is)"
