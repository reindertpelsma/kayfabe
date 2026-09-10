#!/usr/bin/env bash
# ★★★★★ w409 — THE LLM, on the tree where the raw client passes 8/8.
#
# The raw client's mean test is green at threads:8 rounds:8 on two consecutive boots
# (`w407`/`w407b`, rev 444acb1f) after the invalidate stopped completing before its refresh
# ran. The LLM is the lane that was blocked behind it, and it is a DIFFERENT question: the
# client proves ORDERING and CONTENT on three narrow paths, the LLM drives real CUDA at
# volume and can fail for reasons the client cannot express.
#
# ⊘ The grader is `w392_llm.sh` and it does NOT trust a token count. `LLM_TOKENS=16` has
# passed over GARBAGE TEXT before, under greedy decoding, which is why it compares the GPU's
# text against a SAME-BOOT CPU run of the same model. The CPU arm is the correctness oracle,
# never a performance baseline.
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
BENCH=${BENCH_DIR:-/workspace/bench}
tag=${PREFIX:-w409}

export KAYFABE_ISOLATES=real KAYFABE_GUEST_RAM=memfd NVKVM_RAM_BACKEND=memfd \
       KAYFABE_FB_JOIN=shared KAYFABE_GUEST_RING=ring KAYFABE_GR_ROUTE=passthrough \
       KAYFABE_OPERAND_JOIN=join KAYFABE_PT_SWEEP=on \
       KAYFABE_PT_WITNESS_EXEC=on KAYFABE_CE_EXECUTOR=host \
       NVKVM_RAM_MB=${NVKVM_RAM_MB:-16384} BOOT_TIMEOUT=${BOOT_TIMEOUT:-240}
export POST_CAPTURE_HOOK="$SRC_DIR/w392_llm.sh"
export LLM_TIMEOUT=${LLM_TIMEOUT:-1800} LLM_CPU_TIMEOUT=${LLM_CPU_TIMEOUT:-900}

echo "=== w409 LLM BOOT $(date -Is) tag=$tag ==="
# ⚠ CONTENT, never a stamp. `refresh_page_tables` is emitted only by the tree that fixed the
# invalidate; 0 means an older binary is about to be measured.
echo "REFRESH in binary: $(strings "$BENCH/qemu-build/qemu-system-x86_64" 2>/dev/null | grep -c 'MMUINVAL-REFRESH') (0 ⇒ STOP, this predates w406)"
if pgrep -x qemu-system-x86 >/dev/null 2>&1; then echo "⊘ a QEMU is running; refusing"; exit 3; fi

bash "$SRC_DIR/boot_capture.sh" "$tag" > "$BENCH/run_${tag}_driver.log" 2>&1
echo "boot_capture rc=$? (⊘ read the artefacts, not the code)"

Q="$BENCH/run_${tag}_qemu.log"; D="$BENCH/run_${tag}_probe.log"
echo "--- LEDGER tag=$tag ---"
echo "[llm]  $(grep -a 'W392_OUTCOME=' "$D" | tail -1 | cut -c1-110)"
grep -aE 'W392_GPU_TOKENS|W392_CPU_TOKENS|W392_GPU_TEXT|W392_CPU_TEXT|W392_MINMM_SUM|W392_XIDS' "$D" | tail -6 | sed 's/^ */[llm]  /' | cut -c1-120
echo "--- ⚠ A TOKEN COUNT IS NOT A PASS: the two texts must MATCH ---"
echo "[xid]  host Xid lines: $(grep -ac 'Xid' "$BENCH/run_${tag}_hostdmesg.log" 2>/dev/null)"
grep -a 'Xid' "$BENCH/run_${tag}_hostdmesg.log" 2>/dev/null | sed 's/.*faulted @/  @/' | cut -c1-70
echo "[trig] MMUINVAL-REFRESH: $(grep -ac 'MMUINVAL-REFRESH' "$Q")  worst refresh_ms: $(grep -ao 'refresh_ms=[0-9.]*' "$Q" | sort -t= -k2 -rn | head -1)"
echo "--- END LEDGER tag=$tag ---"
