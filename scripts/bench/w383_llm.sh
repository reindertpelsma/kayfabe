#!/usr/bin/env bash
# ★★★★★ w383 — THE DOORBELL RUNS ASYNCHRONOUSLY. One variable against w380llm2.
#
# > Owner, 2026-09-06: "Never do a blocking call during a doorbell write, quickly re-enter the
# > VM. Instead if you need to do mappings, run the doorbell asynchronously. There is no
# > guarantee that a real GPU gives either — the doorbell runs immediately in-line, it's a
# > schedule."
#
# ⊘ THE ONE VARIABLE IS `KAYFABE_DOORBELL_ASYNC`. Every other arm is w290p's default, byte for
#   byte, so `off` reproduces w380llm2 and `on` differs from it in exactly this.
#
# ## ★★★ PRE-REGISTERED, BEFORE THE BOOT — all five, so none can be read as the good one after
#   1. LLM_TOKENS > 0            — the milestone. Reported whatever it is, ABSENT included.
#   2. inline_exceptions         — target 0. ⚠ See the note below: 0 is NOT reachable in one
#                                  rung and a non-zero is not automatically a failure.
#   3. worst_trap                — from 1 750 538 us to something a guest timeout tolerates.
#   4. NO CORRECTNESS REGRESSION — SUPERSEDED=0, SUPERSEDE CAPPED=0, host Xid 31=0, coverage
#                                  refusals on the faulting VAS=0. All four must HOLD.
#   5. PUBQUEUE refused=0        — a non-zero means some doorbell ran inline after all, and
#                                  every number above is then a mixture of two arms.
#
# ⚠ **`inline_exceptions=0` CANNOT be reached by moving the doorbell alone, and saying so here
#   is not lowering the bar.** The witness counts host RM verbs issued under the BQL from ANY
#   trap, and `publication_off_the_bql.md` §4 rules that the REVOCATION direction must STAY
#   synchronous — deferring an unmap is a leak window, not a latency choice. `Regs::write`'s
#   budgeted drain therefore issues host verbs on the vCPU by design. The number this arm
#   moves is the DOORBELL's share of it; read the DELTA, not the floor.
#
# ⚠ **A boot that does not reach the workload is UNMEASURED, not a failure**, and `LLM_TOKENS`
#   absent is NOT `LLM_TOKENS=0`. `llm_hook2.sh` prints `ABSENT` for exactly that reason.
set -uo pipefail
REPO=${KAYFABE_REPO:-/root/kayfabe-w383}
export KAYFABE_REPO="$REPO"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w383}
export KAYFABE_TAG=${KAYFABE_TAG:-w383llm}
# ★★★★★ THE RUNG. ⊘ Not defaulted here on purpose — the caller says which arm, so the control
#   and the evidence run are the same command with one word changed.
export KAYFABE_DOORBELL_ASYNC=${KAYFABE_DOORBELL_ASYNC:?set to on|off — the arm is the rung}
export POST_CAPTURE_HOOK=${POST_CAPTURE_HOOK:-$REPO/scripts/bench/llm_hook2.sh}
# ★ Longer than w376's, because w380 measured the workload STILL BEING SERVED seconds before
#   its own `timeout 600` killed it. A kill at the deadline produced no LLM_TOKENS line, and an
#   absent measurement is the one outcome this rung cannot afford to buy again.
export LLM_TIMEOUT=${LLM_TIMEOUT:-1500}
export GQ_TIMEOUT=${GQ_TIMEOUT:-2400}
export CAPTURE_TIMEOUT=${CAPTURE_TIMEOUT:-3600}
export NVKVM_RAM_MB=${NVKVM_RAM_MB:-8192}
echo "=== ★ ARM=[$KAYFABE_DOORBELL_ASYNC] HOOK=[$POST_CAPTURE_HOOK] LLM_TIMEOUT=[$LLM_TIMEOUT] TAG=[$KAYFABE_TAG] ==="
"$REPO/scripts/bench/w290p_run.sh" "${W298_ARM:-drain}"
BRC=$?
OUT=/workspace/${KAYFABE_TAG}.log
PROBE=/workspace/bench/run_${KAYFABE_TAG}_probe.log
Q=/workspace/bench/run_${KAYFABE_TAG}_qemu.log
D=/workspace/bench/run_${KAYFABE_TAG}_hostdmesg.log
echo ""
echo "================================================================================"
echo "=== ★★★★★ W383 GRADING — arm=$KAYFABE_DOORBELL_ASYNC inner_rc=$BRC  $(date -Is)"
echo "================================================================================"
echo "--- ⊘ THE ARM ACTUALLY IN FORCE (a boot happening is not an arm running) ---"
grep -ao "DOORBELL-ASYNC arm=[a-z]*" "$Q" 2>/dev/null | sort | uniq -c | sed 's/^/    /'
echo "    worker STARTED lines  = [$(grep -ac "DOORBELL-ASYNC worker STARTED" "$Q" 2>/dev/null)]"
echo "    worker FAILED lines   = [$(grep -ac "WORKER FAILED TO START" "$Q" 2>/dev/null)]  (MUST be 0)"
echo "    ⊘ SCHEDULED reports   = [$(grep -ac "SCHEDULED \[Pubqueue::" "$Q" 2>/dev/null)]"
echo ""
echo "--- ★ GRADE 1: LLM_TOKENS (ABSENT is UNMEASURED, never 0) ---"
grep -aE "LLM_TOKENS_GRADE|W382_OUTCOME|W382_MINMM|LLM_(TOKENS|OK|TEXT|EXC|MS)=|TORCH_|PROP_CAPABILITY|HOST_XID" "$PROBE" 2>/dev/null | tail -20 | sed 's/^/    /'
echo ""
echo "--- ★ GRADE 2+3: THE TRAP WITNESS (last emission of the boot) ---"
grep -ao "TRAPWITNESS[^|]*" "$Q" 2>/dev/null | tail -1 | sed 's/^/    /'
echo "    ⊘ TRAPWITNESS lines = [$(grep -ac TRAPWITNESS "$Q" 2>/dev/null)] — 0 means the instrument never ran and every number is VACUOUS"
echo ""
echo "--- ★ GRADE 4: NO CORRECTNESS REGRESSION (all four MUST hold) ---"
echo "    SUPERSEDED          = [$(grep -ac "SUPERSEDED" "$Q" 2>/dev/null)]  (MUST be 0)"
echo "    ⊘ SUPERSEDE CAPPED  = [$(grep -ac "SUPERSEDE CAPPED" "$Q" 2>/dev/null)]  (MUST be 0)"
echo "    host Xid lines      = [$(grep -ac "Xid (PCI" "$D" 2>/dev/null)]  (MUST be 0 NEW)"
grep -ao "proc=3 pdb=0x201000[^|]\{0,120\}" "$Q" 2>/dev/null | tail -1 | sed 's/^/    /'
grep -ao "candidates=[0-9]* refused=[0-9]*" "$Q" 2>/dev/null | sort | uniq -c | sort -rn | head -6 | sed 's/^/    /'
echo ""
echo "--- ★ GRADE 5: THE LANE ITSELF ---"
grep -ao "PUBQUEUE[^|]*" "$Q" 2>/dev/null | tail -1 | sed 's/^/    /'
echo "    ⊘ PUBQUEUE lines = [$(grep -ac PUBQUEUE "$Q" 2>/dev/null)] — 0 means the census never printed"
echo "    ⚠ SERVED-LOCALLY OFF THE TRAP = [$(grep -ac "SERVED-LOCALLY OFF THE TRAP" "$Q" 2>/dev/null)]"
echo "    ⊘ VECTOR REFUSED              = [$(grep -ac "VECTOR REFUSED" "$Q" 2>/dev/null)]"
echo ""
echo "--- ⊘ THE PUBLICATION'S OWN COST, unchanged in code and now off the vCPU ---"
echo "    doorbells (plane counter, last):"
grep -ao "doorbells=[0-9]*[^|]*" "$Q" 2>/dev/null | tail -1 | sed 's/^/      /'
echo "    VAS-PUBLISH passes = [$(grep -ac "VAS-PUBLISH token=" "$Q" 2>/dev/null)]"
echo "    publication wall   = [$(grep -ao "published=[0-9]* refused=[0-9]* in [0-9]* ms" "$Q" 2>/dev/null | grep -o "in [0-9]* ms" | awk '{s+=$2} END {print s"ms over "NR" passes"}')]"
echo ""
echo "--- ★★ HARNESS SELF-CHECK — assert THIS block's own inputs exist ---"
echo "    probe bytes   = [$(wc -c < "$PROBE" 2>/dev/null)]"
echo "    qemu-log byte = [$(wc -c < "$Q" 2>/dev/null)]"
echo "    run-log bytes = [$(wc -c < "$OUT" 2>/dev/null)]"
echo "    grading lines = [$(grep -ac "LLM_TOKENS_GRADE" "$PROBE" 2>/dev/null)]  (MUST be >= 1)"
echo "    ⊘ zero bytes is not 'not yet'; it is a state that needs its own check."
echo "=== W383 EXIT rc=$BRC at $(date -Is) ==="
