#!/usr/bin/env bash
# After queue2.sh: repeat of the doorbell +20 us arm whose second Steel Nomad run produced no result file.
ROOT=/data/claude-trapexp-20261019; cd $ROOT/scripts
while ! grep -q QUEUE2_DONE $ROOT/queue2.log; do sleep 5; done
BASE="KF3_LIFECYCLE_COUNT=1 KF3_LIFECYCLE_PERIOD_S=5"
echo "ARM_START db20b $(date -Is)"
python3 arm.py --arm db20b --rev 02911cbb --flags "$BASE KF3_DIAG_TRAP_DELAY_DB_US=20" --n 4108 > ../arms_db20b.out 2>&1 < /dev/null
echo "ARM_DONE db20b rc=$? $(date -Is)"
echo QUEUE3_DONE $(date -Is)
