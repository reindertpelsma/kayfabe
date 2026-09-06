#!/usr/bin/env bash
# ★★★★★ The LLM workload, run INSIDE the Mode-2 guest, against our emulated GPU.
#
# ## ★★★ PRE-REGISTERED, BEFORE THE BOOT — every outcome, so none reads as the favourable one
#   (A) `LLM_TOKENS` > 0 AND `LLM_OK=1`  ⇒ ★★★★★ LLM COMPUTE. A token count is un-forgeable in
#       the same way `43` is: no copy, fill, or completion we wrote ourselves produces one.
#       Report the text and EVERY relaxation that was on; it is a relaxed green until they come off.
#   (B) `LLM_TOKENS=0` with `LLM_EXC=`   ⇒ the workload REACHED the GPU and the GPU leg failed.
#       ★ THE MOST DIAGNOSTIC OUTCOME. Name the exception and the stage.
#   (C) `TORCH_CUDA_AVAILABLE=False`     ⇒ libcuda never saw a device. This is an INIT failure,
#       not a compute failure — do not read it as the data plane.
#   (D) NO `LLM_TOKENS=` line at all     ⇒ ⊘ THE MEASUREMENT DID NOT HAPPEN. It is NOT 0.
#       Say where the ladder stopped. `RUNNER_PRESENT=no` attributes it to the guest image.
#   (E) guest unreachable                ⇒ ⊘ UNMEASURED, and it is the harness's problem.
#
# ⚠ The CPU control already PASSED during provisioning (provision_guest_llm.sh refuses to
# finish without it), so a failure here is OURS and not a bad wheel or a truncated model.
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
G="$SRC_DIR/gssh_nv"
NTOK=${LLM_NTOK:-16}
TMO=${LLM_TIMEOUT:-600}

echo "=== ★★★★★ LLM ON THE EMULATED GPU  ntok=$NTOK ==="
if ! $G true >/dev/null 2>&1; then
  echo "LLM_OUTCOME=(E) UNMEASURED_GUEST_UNREACHABLE"; exit 0
fi

echo "--- guest preconditions (an absence here attributes the failure to the IMAGE) ---"
RUNNER=$($G 'test -f /opt/llm/run_llm.py && echo yes || echo no' 2>&1 | tr -d '\r')
VENV=$($G 'test -x /home/ubuntu/llmvenv/bin/python && echo yes || echo no' 2>&1 | tr -d '\r')
echo "RUNNER_PRESENT=$RUNNER"
echo "VENV_PRESENT=$VENV"
echo "SMI=$($G 'nvidia-smi --query-gpu=name --format=csv,noheader 2>&1 | head -1' 2>&1 | tr -d '\r')"
if [ "$RUNNER" != yes ] || [ "$VENV" != yes ]; then
  echo "LLM_OUTCOME=(D) UNMEASURED_IMAGE_MISSING_WORKLOAD — run provision_guest_llm.sh"; exit 0
fi

echo "--- generate on device=cuda ---"
OUT=$($G "cd /opt/llm && HF_HOME=/opt/llm/hf LLM_DEVICE=cuda LLM_NTOK=$NTOK \
          timeout $TMO /home/ubuntu/llmvenv/bin/python run_llm.py 2>&1" 2>&1 | tr -d '\r')
echo "$OUT" | sed 's/^/    /'

# ⊘ Grade on the LINES, not on the exit status — a timeout kills the process and its status
#    says nothing about how far the workload got.
TOKENS=$(echo "$OUT" | sed -n 's/^LLM_TOKENS=//p' | tail -1)
OK=$(echo "$OUT"     | sed -n 's/^LLM_OK=//p'     | tail -1)
AVAIL=$(echo "$OUT"  | sed -n 's/^TORCH_CUDA_AVAILABLE=//p' | tail -1)
EXC=$(echo "$OUT"    | sed -n 's/^LLM_EXC=//p'    | tail -1)

echo ""
echo "=== ★★★★★ THE VERDICT, stated once, in the pre-registered vocabulary"
if [ -z "$TOKENS" ]; then
  echo "    (D) ⊘ UNMEASURED — no LLM_TOKENS= line. NOT 0, NOT a failure value."
  echo "        last lines above are where the ladder stopped."
elif [ "$TOKENS" -gt 0 ] 2>/dev/null && [ "${OK:-0}" = 1 ]; then
  echo "    (A) ★★★★★ LLM COMPUTE ON THE EMULATED GPU. tokens=$TOKENS — un-forgeable."
elif [ "${AVAIL:-}" = "False" ]; then
  echo "    (C) libcuda saw NO device (TORCH_CUDA_AVAILABLE=False) — an INIT failure."
  echo "        ⊘ do not read this as the data plane."
else
  echo "    (B) reached the GPU and the GPU leg FAILED: LLM_EXC=${EXC:-<none>}"
  echo "        ★ the most diagnostic outcome available."
fi
echo "LLM_TOKENS_GRADE=${TOKENS:-ABSENT}"
