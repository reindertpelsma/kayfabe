#!/usr/bin/env bash
# After queue2.sh: a POSITIVE CONTROL for the delay method (interrupt-tree +100 us: does Steel Nomad respond to ANY trap cost at all?),
# skipped when the time box is near. Probe repetitions reduced to 5 for time (stated in the report).
ROOT=/data/claude-trapexp-20261019; cd $ROOT/scripts
while ! grep -q QUEUE2_DONE $ROOT/queue2.log; do sleep 5; done
BASE="KF3_LIFECYCLE_COUNT=1 KF3_LIFECYCLE_PERIOD_S=5"
if [ "$(date +%s)" -gt "$(date -d '2026-10-19 21:50' +%s)" ]; then echo "SKIP intr100 (time box)"; echo QUEUE3_DONE $(date -Is); exit 0; fi
echo "ARM_START intr100 $(date -Is)"
python3 arm.py --arm intr100 --rev 02911cbb --flags "$BASE KF3_DIAG_TRAP_DELAY_INTR_US=100" --probe-reps 5 --n 4108 > ../arms_intr100.out 2>&1 < /dev/null
echo "ARM_DONE intr100 rc=$? $(date -Is)"
echo QUEUE3_DONE $(date -Is)
