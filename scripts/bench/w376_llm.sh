#!/usr/bin/env bash
# ★★★★★ w376 — the LLM on the emulated GPU. Same shape as w297_cup3.sh, different workload.
# ⊘ The arming does NOT move from w297's: changing the workload AND the arming in one step
#   would make the outcome unattributable. See llm_hook.sh for the pre-registered outcomes.
set -uo pipefail
REPO=${KAYFABE_REPO:-/root/kayfabe}
export KAYFABE_REPO="$REPO"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w297}
export KAYFABE_TAG=${KAYFABE_TAG:-w376llm}
export POST_CAPTURE_HOOK="$REPO/scripts/bench/llm_hook.sh"
export GQ_TIMEOUT=${GQ_TIMEOUT:-1800}
"$REPO/scripts/bench/w290p_run.sh" "${W298_ARM:-drain}"
BRC=$?
OUT=/workspace/${KAYFABE_TAG}.log
echo ""
echo "================================================================================"
echo "=== ★★★★★ W376 GRADING — LLM on the emulated GPU  inner_rc=$BRC  $(date -Is)"
echo "================================================================================"
grep -aE "LLM_OUTCOME|LLM_TOKENS_GRADE|THE VERDICT|^    \([A-E]\)|RUNNER_PRESENT|VENV_PRESENT|SMI=" "$OUT" 2>/dev/null | tail -20
echo "--- ⊘ EVERY RELAXATION THAT WAS ON — a relaxed green is a MAP, not the milestone ---"
for v in KAYFABE_PT_SWEEP KAYFABE_OPERAND_JOIN KAYFABE_FB_JOIN KAYFABE_VAS_PUBLISH \
         KAYFABE_GR_ROUTE KAYFABE_GUEST_RING KAYFABE_ISOLATES KAYFABE_CE_EXECUTOR; do
  echo "    $v = [$(grep -aoE "$v=[a-z]+" "$OUT" 2>/dev/null | tail -1)]"
done
echo "--- ★★ HARNESS SELF-CHECK — assert THIS block's own output exists ---"
echo "    w376 grading lines = [$(grep -ac "LLM_TOKENS_GRADE" "$OUT" 2>/dev/null)]  (MUST be >= 1)"
echo "    log bytes          = [$(wc -c < "$OUT" 2>/dev/null)]"
echo "    ⊘ zero bytes is not 'not yet'; it is a state that needs its own check."
echo "=== W376 EXIT rc=$BRC at $(date -Is) ==="
