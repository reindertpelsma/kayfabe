#!/usr/bin/env bash
# ★★★★★ w376 — the LLM on the emulated GPU. Same shape as w297_cup3.sh, different workload.
# ⊘ The arming does NOT move from w297's: changing the workload AND the arming in one step
#   would make the outcome unattributable. See llm_hook.sh for the pre-registered outcomes.
set -uo pipefail
REPO=${KAYFABE_REPO:-/root/kayfabe}
export KAYFABE_REPO="$REPO"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w297}
export KAYFABE_TAG=${KAYFABE_TAG:-w376llm}
# ⊘ **OVERRIDABLE since w380, and DEFAULTED to w376's own hook so every existing caller
#   behaves byte-identically.** It was an unconditional `export`, so a caller that set
#   POST_CAPTURE_HOOK in its environment got `llm_hook.sh` anyway — silently, with the run log
#   faithfully recording the hook it actually ran. ⇒ Swapping the instrument was NOT
#   EXPRESSIBLE from outside this file, which is the shape that makes an evidence run and its
#   control indistinguishable at the call site (`w298`'s ruling, one script over).
export POST_CAPTURE_HOOK=${POST_CAPTURE_HOOK:-$REPO/scripts/bench/llm_hook.sh}
export GQ_TIMEOUT=${GQ_TIMEOUT:-1800}
# ★ the LLM needs more than the 2 GiB default — w376 run 1 was OOM-killed at 2048.
export NVKVM_RAM_MB=${NVKVM_RAM_MB:-8192}
echo "=== ★ HOOK=[$POST_CAPTURE_HOOK] LLM_TIMEOUT=[${LLM_TIMEOUT:-<default 600>}] LLM_NTOK=[${LLM_NTOK:-<default 16>}] ==="
"$REPO/scripts/bench/w290p_run.sh" "${W298_ARM:-drain}"
BRC=$?
OUT=/workspace/${KAYFABE_TAG}.log
# ⊘ THE HOOK'S OUTPUT IS NOT IN $OUT. boot_capture.sh sends POST_CAPTURE_HOOK stdout to
#    run_<tag>_probe.log, and $OUT carries only "hook finished: rc=0". Grepping $OUT for the
#    verdict therefore finds NOTHING and reads as "the hook produced no result" -- which is
#    what happened on w376's first run. ★ The harness self-check ("grading lines MUST be >= 1")
#    caught it; without that line this would have looked like a silent workload failure.
PROBE=/workspace/bench/run_${KAYFABE_TAG}_probe.log
echo ""
echo "================================================================================"
echo "=== ★★★★★ W376 GRADING — LLM on the emulated GPU  inner_rc=$BRC  $(date -Is)"
echo "================================================================================"
grep -aE "LLM_OUTCOME|LLM_TOKENS_GRADE|THE VERDICT|\([A-E]\)|RUNNER_PRESENT|VENV_PRESENT|SMI=|TORCH_|LLM_(TOKENS|OK|TEXT|EXC|MS)=|Killed" "$PROBE" 2>/dev/null | tail -24
echo "--- ⊘ EVERY RELAXATION THAT WAS ON — a relaxed green is a MAP, not the milestone ---"
for v in KAYFABE_PT_SWEEP KAYFABE_OPERAND_JOIN KAYFABE_FB_JOIN KAYFABE_VAS_PUBLISH \
         KAYFABE_GR_ROUTE KAYFABE_GUEST_RING KAYFABE_ISOLATES KAYFABE_CE_EXECUTOR; do
  echo "    $v = [$(grep -aoE "$v=[a-z]+" "$OUT" 2>/dev/null | tail -1)]"
done
echo "--- ★★ HARNESS SELF-CHECK — assert THIS block's own output exists ---"
echo "    w376 grading lines = [$(grep -ac "LLM_TOKENS_GRADE" "$PROBE" 2>/dev/null)]  (MUST be >= 1)"
echo "    probe bytes        = [$(wc -c < "$PROBE" 2>/dev/null)]"
echo "    run-log bytes      = [$(wc -c < "$OUT" 2>/dev/null)]"
echo "    ⊘ zero bytes is not 'not yet'; it is a state that needs its own check."
echo "=== W376 EXIT rc=$BRC at $(date -Is) ==="
