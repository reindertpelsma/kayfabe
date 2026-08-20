#!/usr/bin/env bash
# ★★★★★ w362 — SEPARATE "SECOND PROCESS" FROM "AFTER nvidia-smi", AND BOUND EVERY STEP.
#
# w361 measured, in ONE boot: A (torch first) PASSED 8/8 completions OBSERVED; C (torch
# after nvidia-smi) got 0/8 and spun forever. But C was BOTH "the second torch process"
# AND "the process after nvidia-smi" — w361 cannot tell those apart. A2 is that control.
#
# ⚠ AND w361 COULD NEVER REPORT D: C hung, `timeout 2400` killed the whole run, so the
#   "is it permanent or one-shot" step never executed. Every step here is BOUNDED and a
#   timeout is reported BY NAME as HUNG — never as a failure, never as silence.
#
# PRE-REGISTERED READINGS (written before the run, per the campaign's own rule):
#   A pass, A2 HUNG/FAIL              ⇒ ANY second CUDA process dies; nvidia-smi irrelevant.
#   A pass, A2 pass, C HUNG/FAIL      ⇒ nvidia-smi specifically poisons the successor.
#   A pass, A2 pass, C pass           ⇒ w361's C failure does NOT reproduce; suspect the
#                                       instrument/boot, not the model. (n=1 is not a grade.)
#   D distinguishes permanent damage from one-shot.
set -uo pipefail
SELF=$(readlink -f "$0")
STEP_TIMEOUT=${STEP_TIMEOUT:-180}

if [ "${W362_ROLE:-}" = hook ]; then
  TAG=${1:?tag}; REPO=${KAYFABE_REPO:?}; G="$REPO/scripts/bench/gssh_nv"
  echo "=== w362 SEQUENTIAL-PROCESS A/A2/B/C/D tag=$TAG step_timeout=${STEP_TIMEOUT}s ==="
  if ! $G true >/dev/null 2>&1; then echo "W362_OUTCOME=UNMEASURED_GUEST_UNREACHABLE"; exit 0; fi
  $G "cat > /tmp/t.py" <<'PYEOF'
import sys, torch
tag = sys.argv[1]
try:
    torch.cuda.init()
    t = torch.ones(4, device="cuda")
    print("%s TORCH_OK avail=%s count=%d sum=%s" % (tag, torch.cuda.is_available(), torch.cuda.device_count(), t.sum().item()))
except Exception as ex:
    print("%s TORCH_FAIL avail=%s %s: %s" % (tag, torch.cuda.is_available(), type(ex).__name__, str(ex).split('\n')[0]))
PYEOF
  # ⊘ A step that produced no line is NOT a pass and NOT a fail — it is HUNG, and it must
  #   say so itself. Silence is the one outcome that reads as benign while meaning nothing.
  torch_step() {
    local nm=$1 out rc
    out=$($G "timeout ${STEP_TIMEOUT} /opt/llm/venv/bin/python /tmp/t.py $nm 2>&1 | grep -E 'TORCH_(OK|FAIL)'" 2>&1 | tr -d '\r')
    rc=$?
    if [ -n "$out" ]; then echo "$out"; else echo "$nm TORCH_HUNG (no verdict line in ${STEP_TIMEOUT}s, harness rc=$rc)"; fi
  }
  echo "--- A: torch FIRST, nothing before it (predicted PASS) ---";           torch_step A
  echo "--- A2: torch AGAIN, NO nvidia-smi between — ★ THE CONTROL w361 LACKED ---"; torch_step A2
  echo "--- B: nvidia-smi runs and EXITS ---"
  $G "timeout ${STEP_TIMEOUT} nvidia-smi --query-gpu=name --format=csv,noheader 2>&1 | head -1; echo smi_rc=\$?" 2>&1 | tr -d '\r'
  echo "--- C: torch after the nvidia-smi predecessor ---";                    torch_step C
  echo "--- D: once more — permanent or one-shot? ---";                        torch_step D
  echo "=== w362 STEPS DONE ==="
  exit 0
fi
case "${1:-}" in run) ;; *) echo "usage: $0 run" >&2; exit 64 ;; esac
REPO=${KAYFABE_REPO:?}
export KAYFABE_REPO="$REPO" KAYFABE_TAG=${KAYFABE_TAG:-w362seq} W362_ROLE=hook STEP_TIMEOUT
export POST_CAPTURE_HOOK="$SELF" GQ_TIMEOUT=${GQ_TIMEOUT:-900}
rm -f /workspace/bench/qemu-build/qemu-system-x86_64
"$REPO/scripts/bench/w290p_run.sh" "${W362_ARM:-drain}"
