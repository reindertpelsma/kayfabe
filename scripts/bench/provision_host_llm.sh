#!/usr/bin/env bash
# ★★★★★ THE HOST-SIDE LLM BASELINE — the denominator goal 9's "parity" has never had.
#
# Goal 9: *"the LLM working, then at parity"*. `[measured w710]` the LLM **works** in the guest
# (`LLM_OK=1 LLM_TOKENS=16`). ⊘ Parity is a **RATIO**, and until this script there was no host
# number to divide by — so "at parity" was unmeasurable, not merely unmet.
#
# ## ★★ What makes the ratio legitimate, and what would silently break it
#
# It installs **`scripts/bench/run_llm.py`, the same file the guest runs** (w713b extracted it from
# the guest provisioner's heredoc for exactly this reason). Same prompt, same `NTOK`, same greedy
# decode, same `LLM_MS` enclosing `generate()` alone. ⊘ Two copies would drift in prompt length,
# token count, dtype or what the timer encloses, and the ratio would compare two different
# measurements while looking like one.
#
# ⚠ **The model must be the same weights, not merely the same name.** Both sides resolve
# `Qwen/Qwen2-0.5B-Instruct` from the HF cache; a host that silently re-downloaded a different
# revision would move the denominator. `MODEL_SHA` is printed on both sides for that reason.
#
# ## ⊘ What this baseline is NOT
#
# - **Not bare metal in the strict sense.** It runs on the vast box's host OS against the real
#   GPU — no VM, no emulated device — which is the right comparison for "what does kayfabe cost",
#   and is NOT a statement about the card's peak throughput.
# - **Not a throughput benchmark.** One prompt, `NTOK` tokens, greedy. It measures the same thing
#   the guest measures, which is the only property a ratio needs.
# - **Not valid across machines.** The denominator belongs to THIS box. A guest number from one
#   rental divided by a host number from another is not a parity figure.
set -uo pipefail
say() { printf '\n=== %s\n' "$*"; }
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
MODEL=${LLM_MODEL:-Qwen/Qwen2-0.5B-Instruct}
NTOK=${LLM_NTOK:-16}
ROOT=${HOST_LLM_ROOT:-/opt/llm-host}

say "host LLM baseline: model=$MODEL ntok=$NTOK root=$ROOT"

if ! command -v python3 >/dev/null 2>&1; then
  echo "HOSTLLM_RC=2 ⊘ no python3 on the host"; exit 2
fi

mkdir -p "$ROOT" || { echo "HOSTLLM_RC=2 ⊘ cannot create $ROOT"; exit 2; }
if [ ! -x "$ROOT/venv/bin/python" ]; then
  say "creating the venv (once)"
  python3 -m venv "$ROOT/venv" || { echo "HOSTLLM_RC=2 ⊘ venv failed"; exit 2; }
  # ⊘ CUDA wheels, not CPU: a CPU torch here would produce a baseline that never touches the GPU
  # and a ratio that flatters kayfabe by comparing it against a slower thing.
  "$ROOT/venv/bin/pip" -q install --upgrade pip >/dev/null 2>&1
  "$ROOT/venv/bin/pip" -q install torch transformers accelerate 2>&1 | tail -3
fi

# ★ THE SAME RUNNER THE GUEST USES. Installed, never re-written.
cp "$SRC_DIR/run_llm.py" "$ROOT/run_llm.py" || { echo "HOSTLLM_RC=2 ⊘ runner missing"; exit 2; }

say "pre-caching the model so the graded run is not a network test"
HF_HOME="$ROOT/hf" "$ROOT/venv/bin/python" -c "
from transformers import AutoModelForCausalLM, AutoTokenizer
AutoTokenizer.from_pretrained('$MODEL'); AutoModelForCausalLM.from_pretrained('$MODEL')
print('MODEL_CACHED=yes')" 2>&1 | tail -2

say "★★★★★ THE BASELINE — same runner, same prompt, same NTOK, on the real GPU"
OUT=$(cd "$ROOT" && HF_HOME="$ROOT/hf" LLM_DEVICE=cuda LLM_NTOK="$NTOK" LLM_MODEL="$MODEL" \
        "$ROOT/venv/bin/python" run_llm.py 2>&1)
echo "$OUT" | grep -E '^LLM_' | sed 's/^/    HOST_/'

MS=$(echo "$OUT" | sed -n 's/^LLM_MS=//p' | tail -1)
TOK=$(echo "$OUT" | sed -n 's/^LLM_TOKENS=//p' | tail -1)
DEV=$(echo "$OUT" | sed -n 's/^LLM_DEVICE=//p' | tail -1)

# ⊘ The device is ASSERTED, not assumed — the same gap w712 closed in the guest hook, where the
# CPU control also returns LLM_OK=1 with tokens. A CPU baseline would be a slower denominator and
# would make kayfabe look closer to parity than it is.
if [ "${DEV:-}" != "cuda" ]; then
  echo "HOSTLLM_RC=3 ⊘ baseline ran on device=${DEV:-<absent>}, not cuda — NOT a GPU baseline."
  exit 3
fi
if [ -n "$MS" ] && [ -n "$TOK" ] && [ "$TOK" -gt 0 ] 2>/dev/null; then
  RATE=$(awk -v t="$TOK" -v m="$MS" 'BEGIN{ if (m>0) printf "%.2f", t*1000.0/m; else print "inf" }')
  echo ""
  echo "HOST_LLM_TOK_PER_S=$RATE   (generate() wall only — the SAME basis the guest hook reports)"
  echo "HOSTLLM_RC=0"
  echo "★ parity = guest LLM_TOK_PER_S / $RATE, both from run_llm.py on this box."
else
  echo "HOSTLLM_RC=1 ⊘ no usable baseline (LLM_MS=${MS:-<absent>} LLM_TOKENS=${TOK:-<absent>})"
  exit 1
fi
