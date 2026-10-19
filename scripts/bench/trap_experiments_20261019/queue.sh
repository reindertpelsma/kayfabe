#!/usr/bin/env bash
# Runs the remaining arms back to back (caller holds the GPU lock). Stops early when HARD_STOP (epoch) is near.
ROOT=/data/claude-trapexp-20261019; cd $ROOT/scripts
BASE="KF3_LIFECYCLE_COUNT=1 KF3_LIFECYCLE_PERIOD_S=5"
while pgrep -f "arm.py --arm control1" >/dev/null; do sleep 5; done
i=4101
for spec in "spin|02911cbb|KF3_DIAG_INTR_SPIN=1 KF3_DIAG_INTR_SPIN_CPU=23" \
            "intr20|02911cbb|KF3_DIAG_TRAP_DELAY_INTR_US=20" \
            "db20|02911cbb|KF3_DIAG_TRAP_DELAY_DB_US=20" \
            "intr10|02911cbb|KF3_DIAG_TRAP_DELAY_INTR_US=10" \
            "db10|02911cbb|KF3_DIAG_TRAP_DELAY_DB_US=10" \
            "control2|02911cbb-ctl|"; do
  IFS='|' read -r arm rev fl <<<"$spec"
  if [ "$(date +%s)" -gt "${HARD_STOP:?}" ]; then echo "SKIP $arm (time box)"; continue; fi
  echo "ARM_START $arm $(date -Is)"
  python3 arm.py --arm $arm --rev $rev --flags "$BASE $fl" --n $i > ../arms_$arm.out 2>&1 < /dev/null
  echo "ARM_DONE $arm rc=$? $(date -Is)"
  i=$((i+1)); sleep 5
done
echo QUEUE_DONE $(date -Is)
