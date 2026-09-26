#!/usr/bin/env bash
# ★★★★★ w319 — the four remaining arms, IN SEQUENCE, one launcher.
#
# ⚠ ONE launcher on purpose. `[measured w317]` two detached batches were launched for the same
# work; they `pkill`ed each other's QEMU and produced a full log of `⊘UNMEASURED` rows in 50 s,
# and only the TIMESTAMPS (7 s apart, against 2.5 min/boot) revealed it. GPU runs are STRICTLY
# SERIAL, so the serialization lives here rather than in four hopeful invocations.
#
# Arms, exactly as `traces/w319_intermittent/PREREGISTERED.md` §4 committed them:
#   X-off  ROW_LIMIT=11800                      n=2  PREDICTED 2/2 RED   (same-binary control)
#   X-on   ROW_LIMIT=11800 + COMPLETION_PIN=on  n=3  PREDICTED 3/3 GREEN (the fix under provocation)
#   M      BUDGET_MS=2500                       n=2  PREDICTED 2/2 RED   (the faithful clock knob)
#   H      BUDGET_MS=20000                      n=3  PREDICTED complete=true 3/3
set -uo pipefail
TREE=${1:-/workspace/kayfabe_w319}
A="$TREE/scripts/bench/w319_arm.sh"
echo "=== W319 ALL START $(date -Is) tree=$TREE HEAD=$(cd "$TREE" && git rev-parse --short HEAD)"

bash "$A" "$TREE" w319xoff 2 KAYFABE_VAS_DRAIN_ROW_LIMIT=11800
echo "=== arm X-off done $(date -Is)"
bash "$A" "$TREE" w319xon  3 KAYFABE_VAS_DRAIN_ROW_LIMIT=11800 KAYFABE_COMPLETION_PIN=on
echo "=== arm X-on done $(date -Is)"
bash "$A" "$TREE" w319m    2 KAYFABE_VAS_DRAIN_BUDGET_MS=2500
echo "=== arm M done $(date -Is)"
bash "$A" "$TREE" w319h    3 KAYFABE_VAS_DRAIN_BUDGET_MS=20000
echo "=== arm H done $(date -Is)"

echo "=== W319 ALL TERMINATOR rc=0 $(date -Is)"
