#!/usr/bin/env bash
# ★★★★★ w370 — IS THE RESOURCE CONSUMED BY *OPENING* THE DEVICE, OR BY CREATING A CONTEXT?
#
# MEASURED w367 + w369, two boots, two different process mixes: the FIFTH GPU-touching
# process is the first to fail. In w369 that process was a bare `nvidia-smi`, which
# answered "No devices were found" -- so whatever runs out is NOT a CUDA context and NOT
# torch-specific.
#
# This arm is the cheapest possible discriminator: run `nvidia-smi` EIGHT times, nothing
# else. No CUDA, no torch, no context, no allocation.
#   dies at #5      => the resource is consumed by DEVICE OPEN alone. Prime suspect
#                      DEFAULT_POOL_WORKERS=4 (kayfabe-isolate) -- a hypothesis, NOT yet
#                      evidenced: the log names no exhaustion anywhere.
#   all 8 clean     => device-open does NOT consume it; creating a CONTEXT does, and the
#                      pool-of-4 reading is REFUTED. Then re-run with 8 torch inits.
#   dies elsewhere  => "five" was a coincidence of two boots and the whole ordinal framing
#                      needs rebuilding. n=2 is not a law.
#
# ⊘ Each iteration prints its own index and verdict. A step that produces no line is HUNG
#   and says so -- silence must never read as success.
set -uo pipefail
SELF=$(readlink -f "$0")
STEP_TIMEOUT=${STEP_TIMEOUT:-120}
if [ "${W370_ROLE:-}" = hook ]; then
  TAG=${1:?tag}; REPO=${KAYFABE_REPO:?}; G="$REPO/scripts/bench/gssh_nv"
  echo "=== w370 DEVICE-OPEN SWEEP tag=$TAG (nvidia-smi x8, NO cuda) ==="
  if ! $G true >/dev/null 2>&1; then echo "W370_OUTCOME=UNMEASURED_GUEST_UNREACHABLE"; exit 0; fi
  for i in 1 2 3 4 5 6 7 8; do
    out=$($G "timeout ${STEP_TIMEOUT} nvidia-smi --query-gpu=name --format=csv,noheader 2>&1 | head -1" 2>&1 | tr -d '\r')
    if [ -n "$out" ]; then echo "SMI#$i => $out"; else echo "SMI#$i => ⊘ HUNG (no line in ${STEP_TIMEOUT}s)"; fi
  done
  echo "--- ★ and NOW a torch full init, after 8 opens ---"
  $G "cat > /tmp/t370.py" <<'PYEOF'
import torch
try:
    torch.cuda.init(); t = torch.ones(4, device="cuda")
    print("POST8 TORCH_OK avail=%s sum=%s" % (torch.cuda.is_available(), t.sum().item()))
except Exception as ex:
    print("POST8 TORCH_FAIL avail=%s %s: %s" % (torch.cuda.is_available(), type(ex).__name__, str(ex).split('\n')[0]))
PYEOF
  o=$($G "timeout 600 /opt/llm/venv/bin/python /tmp/t370.py 2>&1 | grep -E 'POST8'" 2>&1 | tr -d '\r')
  [ -n "$o" ] && echo "$o" || echo "POST8 TORCH_HUNG (no verdict in 600s)"
  echo "=== w370 STEPS DONE ==="
  exit 0
fi
case "${1:-}" in run) ;; *) echo "usage: $0 run" >&2; exit 64 ;; esac
REPO=${KAYFABE_REPO:?}
export KAYFABE_REPO="$REPO" KAYFABE_TAG=${KAYFABE_TAG:-w370seq} W370_ROLE=hook STEP_TIMEOUT
export POST_CAPTURE_HOOK="$SELF" GQ_TIMEOUT=${GQ_TIMEOUT:-900}
rm -f /workspace/bench/qemu-build/qemu-system-x86_64
"$REPO/scripts/bench/w290p_run.sh" "${W370_ARM:-drain}"
