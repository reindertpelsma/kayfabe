#!/usr/bin/env bash
# ★★★★★ RE-RUN ONLY THE TIMEOUTS, AT A LONG BUDGET — because one word was hiding three defects.
#
# `[measured w823]` the 45 s suite scored `--engines`, `--concurrency` and `--defer-liveness` as
# the same thing: `TIMEOUT`. At 300 s they are **three different defects**:
#
#     --engines        FAIL 114s   a stranded-token correctness bug, hidden behind the clock
#     --concurrency    TIMEOUT     genuinely HUNG — 6.7x the budget and still nothing
#     --defer-liveness PASS  53s   merely SLOW (14 s on bare metal ⇒ 3.8x)
#
# ⊘ **"A timeout IS a crash" is the right GATE and a terrible DIAGNOSIS.** Keep the budget — it is
# what stops a slow path from being scored green, and what keeps one arm from poisoning the next.
# But a scoreboard that merges *a correctness bug*, *a hang* and *a 3.8x slowdown* into one word is
# unreadable exactly where it matters, which `fast_suite.sh` already says about its OWN verdict
# vocabulary. This script is the second pass that separates them.
#
# ⚠ **It does NOT relax the gate.** The arms stay RED in the suite result. This answers *how* red.
#
#   usage: diagnose_timeouts.sh <suite-tag> [long-budget]
#          diagnose_timeouts.sh w823 300
set -uo pipefail
cd "$(dirname "$0")/../.." || exit 2
BENCH=${BENCH_DIR:-/workspace/bench}
TAG=${1:?usage: diagnose_timeouts.sh <suite-tag> [long-budget]}
LONG=${2:-300}
OUT=$BENCH/${TAG}_suite.out

[ -s "$OUT" ] || { echo "diagnose_timeouts: ⊘ no suite output at $OUT — run fast_suite.sh first"; exit 2; }

# ⊘ Read the ARMS from the suite's own output, never a hand-kept list: a list drifts from the
# suite it describes, and this tree has paid for that ("a gate that is a list is the defect it
# guards").
mapfile -t ARMS < <(awk '$2=="TIMEOUT" {print $1}' "$OUT")
if [ "${#ARMS[@]}" -eq 0 ]; then
    echo "diagnose_timeouts: ✔ no TIMEOUT arms in $OUT — nothing to separate."
    exit 0
fi
echo "diagnose_timeouts: ${#ARMS[@]} timing-out arm(s) from $OUT, re-running at ${LONG}s"
printf '   %s\n' "${ARMS[@]}"
echo "⊘ These arms remain RED in $OUT. This pass answers HOW red, not WHETHER."
bash "$(dirname "$0")/fast_suite.sh" "${TAG}_diag" "$LONG" "${ARMS[@]}"

echo
echo "=== the forwarding gate's OWN rule, per arm ==="
# ⊘⊘ Use `stranded_tokens.awk`, NEVER a fresh grep. `[measured w823]` a hand-written
# `grep 'DOORBELL-LEDGER tok='` over the whole log counts SYSTEM tokens (`0x0001xxxx`), which are
# `forwarded=0` BY DESIGN, and reports identical numbers for passing and failing arms alike.
# The gate is scoped to guest tokens `0x000000xx`. One statement of the rule, in one file.
for arm in "${ARMS[@]}"; do
    a=${arm#--}
    q=$BENCH/fast_${TAG}_diag_${a}_qemu.log
    [ -s "$q" ] || continue
    rows=$(grep -ao 'DOORBELL-LEDGER tok=0x000000[0-9a-f][0-9a-f] .*' "$q" | sort -u)
    gt=$(printf '%s\n' "$rows" | grep -c 'tok=' || true)
    st=$(printf '%s\n' "$rows" | awk -f "$(dirname "$0")/stranded_tokens.awk" | grep -c 'tok=' || true)
    og=$(grep -ao 'OPERAND-GATE refused=[0-9]*' "$q" | tail -1 | grep -o '[0-9]*$')
    printf '%-24s guest_tokens=%s stranded=%s operand_refused=%s\n' "$a" "$gt" "$st" "${og:-0}"
done
