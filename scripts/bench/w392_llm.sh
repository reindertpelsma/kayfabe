#!/usr/bin/env bash
# ★★★★★ w392 — THE LLM, GRADED ON ITS TEXT AGAINST A SAME-BOOT CPU ORACLE.
#
# ## ⊘ Why the existing grade cannot be trusted, measured
#
# `provision_guest_llm.sh:49` states the old contract in words: *"LLM_TOKENS is the whole
# grade — a token count cannot be forged by a copy, a fill, or a completion we wrote
# ourselves."* **That is false, and w386 measured it false**: a boot reported
# `LLM_TOKENS=16` over TEXT THAT WAS GARBAGE. Under `do_sample=False` the model emits
# exactly `max_new_tokens` whatever the numerics are, so the count is a function of the
# ARGUMENT, not of the computation. A wrong matmul produces 16 wrong tokens and scores
# identically to 16 right ones.
#
# ⇒ **The count says the pipeline RAN. Only the text says it ran CORRECTLY.**
#
# ## ★★★ Why the reference is a SAME-BOOT CPU run and not a hardcoded string
#
# Hardcoding `"Paris"` would bake in a claim about one model's greedy output that nobody
# measured, and it would go stale silently the first time the model or the tokenizer
# changed — the `a_capture_derived_table_expires_as_a_vendor_regression` shape.
#
# The CPU run is the same weights, the same tokenizer, the same prompt and the same greedy
# decode, differing in EXACTLY the thing under test: whether the arithmetic happened on the
# host GPU through our emulated device. So it is a **differential**, and it is generated
# fresh in the boot it grades.
# ⊘ The provisioning script already ran a CPU control and THREW ITS TEXT AWAY. That is the
#   discarded-oracle shape; this keeps it.
#
# ## The outcomes, pre-registered
#
#   (P) tokens>0 AND gpu_text == cpu_text  → PASS. The GPU did the arithmetic correctly.
#   (F) tokens>0 AND gpu_text != cpu_text  → ★★★ FORGED-PASS CAUGHT. The old grade would
#                                            have said PASS. This is w386's exact case.
#   (E) no LLM_TOKENS line at all          → ⊘ UNMEASURED. NOT zero, NOT a failure.
#   (C) the CPU oracle itself failed       → ⊘ UNGRADABLE. Without a reference the GPU
#                                            run's text cannot be judged, and saying
#                                            "PASS because tokens>0" is the defect above.
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
G="$SRC_DIR/gssh_nv"
NTOK=${LLM_NTOK:-16}
TMO=${LLM_TIMEOUT:-900}
CPU_TMO=${LLM_CPU_TIMEOUT:-900}
PY=/home/ubuntu/llmvenv/bin/python

xids() { sudo dmesg 2>/dev/null | grep -c "Xid (PCI" || echo 0; }

echo "=== ★★★★★ w392 LLM — graded on TEXT against a same-boot CPU oracle ==="
if ! $G true >/dev/null 2>&1; then echo "W392_OUTCOME=(E) ⊘ UNMEASURED_GUEST_UNREACHABLE"; exit 0; fi

XID_BEFORE=$(xids); echo "HOST_XID_BEFORE=$XID_BEFORE"

# ---- 0. THE SCALE DISCRIMINATOR — the smallest thing that reaches cuBLAS ---------------
# ★★★★★ w392b — WHY THIS RUNS BEFORE THE MODEL.
#
# The w392 LLM boot produced **16 tokens of garbage with ZERO Xids** — it completed, it
# faulted nowhere, and the arithmetic was wrong. `CUP3_VAL=43` passed in the same tree the
# same day. So the failure is NOT "compute is broken" and NOT "a mapping is missing" (a
# missing mapping FAULTS; this did not). It is memory that was **mapped and held the wrong
# bytes**.
#
# A 4x4 fp32 matmul of ones has ONE tiny allocation and no weight transfer. Its sum is 64,
# and 64 is un-forgeable by a copy, a fill or a zero page.
#
#   64 here + garbage from the LLM  ⇒ the defect SCALES with allocation count/size, not
#                                     with the arithmetic path. Look at mapping/publication
#                                     of the large weight buffers.
#   wrong here                      ⇒ the compute path itself is broken, which is a much
#                                     more fundamental and much easier target.
#
# ⊘ Runs BEFORE the model load so it cannot be contaminated by a poisoned context, and its
#   own Xid delta is read separately so "it failed AND faulted" is distinguishable from
#   "it failed cleanly".
# ★★★★★ w424 — THE OPEN ORDINAL, and why this discriminator can now COST the thing it is
# diagnosing.
#
# `[measured w423/w424]` the guest's device node stops opening after a small number of
# processes: opens #1-#3 succeed and **#4 fails with EIO, permanently**, because RM's own
# CeUtils scrubber channel cannot initialise its copy engine on that adapter init
# (`ce_utils.c:304`). ⊘ Every python process below is a device open. This 4x4 run is one, the
# GPU run is the next, and the provisioning that precedes us has already spent some.
#
# ⇒ `LLM_SKIP_4X4=1` moves the GPU run one open earlier. If the GPU arm gets further with the
# discriminator skipped, then the discriminator was consuming the budget that the workload
# needed — a diagnostic that causes the failure it is there to characterise.
#
# ⚠ Skipping it is NOT free: `MINMM_SUM` is what separates *"the arithmetic path is wrong"*
# from *"the small path is right and something else is"*. The grader already reports
# `MINMM_SUM=ABSENT`, and an absent discriminator must be read as UNMEASURED, never as a pass.
if [ "${LLM_SKIP_4X4:-0}" = "1" ]; then
  echo "--- 4x4 scale discriminator ⊘ SKIPPED (LLM_SKIP_4X4=1) — one fewer device open before the GPU run ---"
  echo "    ⊘ MINMM_SUM will read ABSENT below. That is UNMEASURED, not 64 and not a pass."
  MIN=""
  XID_AFTER_MIN=$(xids); echo "HOST_XID_AFTER_MINMM=$XID_AFTER_MIN (no 4x4 was run)"
else
echo "--- 4x4 scale discriminator (before any weight transfer) ---"
MIN=$($G "timeout 300 $PY - <<'PYEOF' 2>&1
import torch
try:
    a = torch.ones(4, 4, device='cuda')
    b = torch.ones(4, 4, device='cuda')
    print('MINMM_SUM=%g' % (a @ b).sum().item())   # 4x4 of ones -> every element 4 -> 64
    print('MINMM_OK=1')
except Exception as e:
    print('MINMM_OK=0'); print('MINMM_EXC=%s: %s' % (type(e).__name__, e))
PYEOF" 2>&1 | tr -d '\r')
echo "$MIN" | sed 's/^/    /'
MINSUM=$(echo "$MIN" | sed -n 's/^MINMM_SUM=//p' | tail -1)
XID_AFTER_MIN=$(xids); echo "HOST_XID_AFTER_MINMM=$XID_AFTER_MIN"
fi

# ---- 1. THE WORKLOAD ON THE GPU (this is the thing under test) -------------------------
echo "--- GPU run ---"
GPU_OUT=$($G "cd /opt/llm && HF_HOME=/opt/llm/hf LLM_DEVICE=cuda LLM_NTOK=$NTOK \
          timeout $TMO $PY run_llm.py 2>&1" 2>&1 | tr -d '\r')
echo "$GPU_OUT" | sed 's/^/    /'
XID_AFTER=$(xids); echo "HOST_XID_AFTER_GPU=$XID_AFTER"

# ---- 2. THE ORACLE, SAME BOOT, SAME EVERYTHING BUT THE DEVICE --------------------------
# ⚠ Run AFTER the GPU arm, deliberately: running it first would warm caches and could mask
#   a GPU failure that only shows on a cold model load. It also means a GPU run that WEDGES
#   the guest is visible as a missing oracle rather than as a silently-skipped comparison.
echo "--- CPU oracle run (same weights, same prompt, same greedy decode) ---"
CPU_OUT=$($G "cd /opt/llm && HF_HOME=/opt/llm/hf LLM_DEVICE=cpu LLM_NTOK=$NTOK \
          timeout $CPU_TMO $PY run_llm.py 2>&1" 2>&1 | tr -d '\r')
echo "$CPU_OUT" | sed 's/^/    /'

# ⊘⊘ **CORRECTED 2026-09-09 (w392llm3) — THIS ANCHOR MISGRADED A PASSING RUN.**
# It was `s/^$2=//p`. The GPU arm's output reaches us INDENTED (the guest-side runner pads its
# lines), so every GPU field read ABSENT and the grader printed `(E) UNMEASURED` over a run that
# had, in the same log:
#     LLM_TEXT= ______. A. Paris B. London C. New York D
#     LLM_TOKENS=16   LLM_OK=1
# i.e. **byte-identical to the CPU oracle**. The CPU arm parsed only because its lines happen to be
# flush-left. ⇒ A grader whose extractor is anchored more tightly than its input reports UNMEASURED
# for SUCCESS — the mirror of `a_count_cannot_see_a_substitution`, and it cost this campaign a
# night of believing the LLM still failed.
pick() { echo "$1" | sed -n "s/^[[:space:]]*$2=//p" | tail -1; }
GPU_TOK=$(pick "$GPU_OUT" LLM_TOKENS); GPU_TXT=$(pick "$GPU_OUT" LLM_TEXT); GPU_RC=$(pick "$GPU_OUT" LLM_RC)
CPU_TOK=$(pick "$CPU_OUT" LLM_TOKENS); CPU_TXT=$(pick "$CPU_OUT" LLM_TEXT); CPU_RC=$(pick "$CPU_OUT" LLM_RC)
GPU_MS=$(pick "$GPU_OUT" LLM_MS);      CPU_MS=$(pick "$CPU_OUT" LLM_MS)

# ★★★★★ tok/s — THE PARITY NUMBER, AND IT WAS ALREADY BEING MEASURED.
#
# ⊘ The campaign note *"nothing in the repo computes tok/s"* was half right in the way that
# matters least: `provision_guest_llm.sh` has ALWAYS printed `LLM_MS`, the wall time around
# `generate()` alone. What was missing was a READER. The measurement existed and no grade
# consumed it — the discarded-oracle shape this file already calls out for the CPU control's
# text, repeated one field over.
# ⚠ Separately true and easy to conflate: the ENV VAR `LLM_MS` is read by nothing (the
#   timeout is `LLM_TIMEOUT`, in SECONDS). The PRINTED `LLM_MS=` line is real data.
#
# ⊘ WHAT THIS NUMBER IS: decode throughput over `generate()` only. It EXCLUDES model load and
# the `.to(device)` weight upload — which, on the Mode-2 path, is exactly where w394 measured
# H2D at ~17 s per 16 MiB. So tok/s here is the FAVOURABLE half of the story and must never be
# quoted as end-to-end parity. Name both or name neither.
toks_per_s() {  # toks_per_s <tokens> <ms>
  awk -v t="${1:-}" -v m="${2:-}" 'BEGIN{
    if (t == "" || m == "" || m+0 <= 0) { print "UNMEASURED" } else { printf "%.3f", t/(m/1000.0) }
  }'
}
GPU_TPS=$(toks_per_s "$GPU_TOK" "$GPU_MS"); CPU_TPS=$(toks_per_s "$CPU_TOK" "$CPU_MS")

echo ""
echo "W392_GPU_TOKENS=${GPU_TOK:-ABSENT}  W392_GPU_RC=${GPU_RC:-ABSENT}"
echo "W392_CPU_TOKENS=${CPU_TOK:-ABSENT}  W392_CPU_RC=${CPU_RC:-ABSENT}"
echo "W392_GPU_TEXT=[${GPU_TXT}]"
echo "W392_CPU_TEXT=[${CPU_TXT}]"
echo "W392_GPU_MS=${GPU_MS:-ABSENT}  W392_GPU_TOKS_PER_S=${GPU_TPS}"
echo "W392_CPU_MS=${CPU_MS:-ABSENT}  W392_CPU_TOKS_PER_S=${CPU_TPS}  ⊘ CPU is a CORRECTNESS oracle, NOT a perf baseline"
echo "W392_TPS_RATIO_GPU_OVER_CPU=$(awk -v g="$GPU_TPS" -v c="$CPU_TPS" 'BEGIN{
  if (g=="UNMEASURED"||c=="UNMEASURED"||c+0==0) print "UNMEASURED"; else printf "%.3f", g/c }')"
echo "    ⊘ decode only — EXCLUDES model load and the .to(device) weight upload."
echo "    ⊘ FOR THIS BOOT'S ARMING (VAS_PUBLISH=drain, PT_SWEEP=on). Quote the arming or don't quote it."
echo "W392_MINMM_SUM=${MINSUM:-ABSENT} (64 = the small path is CORRECT)"
echo "W392_XIDS=${XID_BEFORE}/${XID_AFTER_MIN:-?}/${XID_AFTER} (before/after-4x4/after-gpu)"

echo "=== ★★★★★ THE VERDICT — pre-registered, stated once ==="
if [ -z "${GPU_TOK:-}" ]; then
  echo "    W392_OUTCOME=(E) ⊘ UNMEASURED — the GPU run printed no LLM_TOKENS line at all."
  echo "        ⊘ This is NOT 'zero tokens' and NOT a failure. Read LLM_EXC above."
elif [ -z "${CPU_TOK:-}" ] || [ "${CPU_RC:-1}" != "0" ]; then
  echo "    W392_OUTCOME=(C) ⊘ UNGRADABLE — the CPU ORACLE did not produce a reference."
  echo "        GPU tokens=${GPU_TOK}. ⊘ Grading that as PASS on the count alone is exactly"
  echo "        the w386 defect this hook exists to remove, so it is NOT graded."
elif [ "${GPU_TOK}" -gt 0 ] 2>/dev/null && [ "$GPU_TXT" = "$CPU_TXT" ]; then
  echo "    W392_OUTCOME=(P) ★★★★★ PASS — ${GPU_TOK} tokens AND the text matches the CPU"
  echo "        oracle EXACTLY. The host GPU did the arithmetic, through our emulated device,"
  echo "        correctly. ⊘ Un-forgeable by a copy, a fill or a completion we wrote."
elif [ "${GPU_TOK}" -gt 0 ] 2>/dev/null && [ "${MINSUM:-x}" = "64" ]; then
  echo "    W392_OUTCOME=(F-scale) ★★★★★ FORGED-PASS, AND IT SCALES — ${GPU_TOK} tokens of"
  echo "        WRONG TEXT while the 4x4 matmul is EXACTLY 64. The arithmetic path is"
  echo "        CORRECT at one small allocation and WRONG at the model's. ⇒ the defect is"
  echo "        in mapping/publication of the large buffers, not in compute."
elif [ "${GPU_TOK}" -gt 0 ] 2>/dev/null; then
  echo "    W392_OUTCOME=(F) ★★★ FORGED-PASS CAUGHT — ${GPU_TOK} tokens, WRONG TEXT."
  echo "        4x4 sum=${MINSUM:-ABSENT} (not 64) ⇒ the SMALL path is wrong too, so this is"
  echo "        NOT scale-dependent — a much more fundamental target."
  echo "        The old LLM_TOKENS grade would have called this a PASS (w386, measured)."
  echo "        ⇒ the pipeline RAN and the ARITHMETIC IS WRONG. That is a worse defect than"
  echo "        a refusal, and it is now visible."
else
  echo "    W392_OUTCOME=(Z) the GPU run produced ${GPU_TOK} tokens — it did not generate."
fi
echo "=== w392 llm hook DONE ==="
