#!/usr/bin/env bash
# ★★★★★ w382 — DOES cuBLASLt FAIL ON PROPERTIES, OR ON A POISONED CONTEXT?
#
# The w376 LLM died with:
#   LLM_EXC=RuntimeError: CUDA error: CUBLAS_STATUS_NOT_SUPPORTED
#           when calling `cublasLtMatmulAlgoGetHeuristic(...)`
# and the same boot carried exactly one `Xid 31 FAULT_PDE`. **Two readings fit that, and they
# point at completely different work:**
#
#   (P) PROPERTIES — cuBLASLt picks an algorithm from the device's reported compute
#       capability / SM count / shared-mem-per-block / L2 size. If our emulated device reports
#       those wrong it concludes no algorithm fits. Nothing to do with mappings.
#   (S) STICKY     — CUDA errors are sticky. If the Xid already poisoned the context, the NEXT
#       CUDA call returns an error, often a misleading one. Then cuBLASLt is a SYMPTOM and the
#       fb-join fix is the whole answer.
#
# ⊘ The existing runner logs `TORCH_CUDA_AVAILABLE` and `TORCH_DEV_COUNT` and NOTHING ELSE, so
#   no recorded boot can separate them. This hook adds the two observations that can.
#
# ## ★★★ THE DISCRIMINATOR — a MINIMAL matmul BEFORE anything large can fault
# A 4x4 @ 4x4 fp32 matmul is the smallest thing that reaches cuBLAS. It runs before the model
# is loaded, before any big allocation, before any doorbell storm.
#   - it FAILS with NOT_SUPPORTED, with a CLEAN Xid count  ⇒ (P) PROPERTIES. Chase device caps.
#   - it PASSES and the full model then fails              ⇒ (S) STICKY, or a size/shape wall.
#   - it FAILS with an Xid ALREADY present                 ⇒ ⊘ UNINTERPRETABLE, say so.
#
# ⚠ The Xid count is read BEFORE the probe and AFTER it, from the HOST, because a Mode-2 guest
#   fault surfaces as a host Xid naming `memfd:kayfabe-i`. A count taken only afterwards cannot
#   tell a fault this probe caused from one the boot already had.
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
G="$SRC_DIR/gssh_nv"
NTOK=${LLM_NTOK:-16}
TMO=${LLM_TIMEOUT:-600}

xids() { sudo dmesg 2>/dev/null | grep -c "Xid (PCI" || echo 0; }

echo "=== ★★★★★ w382 CUBLAS DISCRIMINATOR + LLM ==="
if ! $G true >/dev/null 2>&1; then echo "W382_OUTCOME=(E) UNMEASURED_GUEST_UNREACHABLE"; exit 0; fi

XID_BEFORE=$(xids); echo "HOST_XID_BEFORE=$XID_BEFORE"

# ---- the device the guest thinks it has, field by field --------------------------------
echo "--- device properties as the guest sees them ---"
$G "/home/ubuntu/llmvenv/bin/python - <<'PY' 2>&1
import torch
try:
    p = torch.cuda.get_device_properties(0)
    for k in ('name','major','minor','total_memory','multi_processor_count',
              'max_threads_per_multi_processor','warp_size','L2_cache_size',
              'is_integrated','is_multi_gpu_board','regs_per_multiprocessor',
              'shared_memory_per_block','shared_memory_per_multiprocessor'):
        print('PROP_%s=%s' % (k, getattr(p, k, '(absent)')), flush=True)
    print('PROP_CAPABILITY=%d.%d' % (p.major, p.minor), flush=True)
except Exception as e:
    print('PROP_EXC=%s: %s' % (type(e).__name__, e), flush=True)
PY" 2>&1 | tr -d '\r' | sed 's/^/    /'

# ---- the smallest thing that reaches cuBLAS --------------------------------------------
echo "--- MINIMAL 4x4 matmul (before the model, before any large allocation) ---"
MIN=$($G "timeout 180 /home/ubuntu/llmvenv/bin/python - <<'PY' 2>&1
import torch
try:
    a = torch.ones(4, 4, device='cuda')
    b = torch.ones(4, 4, device='cuda')
    c = (a @ b).sum().item()
    print('MINMM_OK=1'); print('MINMM_SUM=%g' % c)   # 4x4 of ones -> every element 4 -> sum 64
except Exception as e:
    print('MINMM_OK=0'); print('MINMM_EXC=%s: %s' % (type(e).__name__, e))
PY" 2>&1 | tr -d '\r')
echo "$MIN" | sed 's/^/    /'
XID_AFTER_MIN=$(xids); echo "HOST_XID_AFTER_MINMM=$XID_AFTER_MIN"

MINOK=$(echo "$MIN" | sed -n 's/^MINMM_OK=//p' | tail -1)
MINSUM=$(echo "$MIN" | sed -n 's/^MINMM_SUM=//p' | tail -1)

# ---- the real workload, unchanged, so the grade stays comparable ------------------------
echo "--- the LLM itself (unchanged from llm_hook.sh) ---"
OUT=$($G "cd /opt/llm && HF_HOME=/opt/llm/hf LLM_DEVICE=cuda LLM_NTOK=$NTOK \
          timeout $TMO /home/ubuntu/llmvenv/bin/python run_llm.py 2>&1" 2>&1 | tr -d '\r')
echo "$OUT" | sed 's/^/    /'
XID_END=$(xids); echo "HOST_XID_AFTER_LLM=$XID_END"

TOKENS=$(echo "$OUT" | sed -n 's/^LLM_TOKENS=//p' | tail -1)
echo ""
echo "W382_MINMM_OK=${MINOK:-NONE}  W382_MINMM_SUM=${MINSUM:-NONE}"
echo "W382_XIDS=${XID_BEFORE}/${XID_AFTER_MIN}/${XID_END} (before/after-minmm/after-llm)"
# ⊘⊘ **`ABSENT`, NOT `0` — corrected at w380, and it is this file's own rule.** `${TOKENS:-0}`
# prints `0` when the runner produced NO `LLM_TOKENS=` line at all, which is exactly the
# conflation `llm_hook.sh`'s pre-registration forbids in words: *"(D) ⊘ THE MEASUREMENT DID NOT
# HAPPEN. It is NOT 0."* `[measured w380llm2]` the LLM was killed by its own `timeout` with no
# token line and this printed `LLM_TOKENS_GRADE=0` — a failure value for an unmeasured one.
echo "LLM_TOKENS_GRADE=${TOKENS:-ABSENT}"
echo "=== ★★★★★ THE VERDICT — pre-registered, stated once"
if [ -z "$MINOK" ]; then
  echo "    W382_OUTCOME=(E) ⊘ UNMEASURED — the minimal matmul printed no MINMM_OK line at all."
elif [ "$XID_BEFORE" != "$XID_AFTER_MIN" ] && [ "$MINOK" = 0 ]; then
  echo "    W382_OUTCOME=(U) ⊘ UNINTERPRETABLE — the minimal matmul faulted AND raised an Xid."
  echo "        Cause and symptom are inseparable in this run. Re-run after the fb-join fix."
elif [ "$MINOK" = 0 ] && [ "$XID_BEFORE" = "$XID_AFTER_MIN" ]; then
  echo "    W382_OUTCOME=(P) ★★★★★ PROPERTIES — cuBLAS refused with a CLEAN Xid count."
  echo "        ⇒ a SECOND wall, independent of mappings: the device we advertise is one"
  echo "        cuBLASLt has no algorithm for. Compare PROP_* above against a real GA106."
elif [ "$MINOK" = 1 ] && [ "${MINSUM:-0}" = "64" ]; then
  echo "    W382_OUTCOME=(S) cuBLAS WORKS on a fresh context (4x4 sum=64, un-forgeable)."
  echo "        ⇒ the w376 CUBLAS_STATUS_NOT_SUPPORTED was a SYMPTOM of the poisoned context,"
  echo "        not a properties wall. The fb-join fix is the whole answer for it."
elif [ "$MINOK" = 1 ]; then
  echo "    W382_OUTCOME=(?) matmul reported OK but sum=${MINSUM} not 64 — ⊘ WRONG ARITHMETIC,"
  echo "        which is worse than a refusal. Do not read this as a pass."
fi
